//! HTTP JSON-RPC client for the Bevy Remote Protocol.
//!
//! Since Bevy 0.16 the Bevy Remote Protocol is JSON-RPC 2.0 over HTTP POST
//! (see `bevy_remote::http`). This client speaks that protocol directly:
//! every request is a POST of `{"jsonrpc":"2.0","id":N,"method":..., "params":...}`
//! to the BRP root URL, and the response is the JSON-RPC response object.

use reqwest::Client;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::server::brp::{builtin_methods, refusal_text, BrpPayload, BrpRequest, BrpResponse};
use crate::server::brp_command_handler::{CommandHandlerRegistry, CoreBrpHandler, BrpCommandHandler};
use crate::server::config::Config;
use crate::server::debug_command_processor::DebugCommandRouter;
use crate::server::error::{Error, Result};

/// Request timeout for a single BRP call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// A game that did not say which process it is (see [`BrpClient::game_instance`]), and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unnamed {
    /// Why the game could not say, in words an agent can act on.
    pub why: String,
    /// The game answered, and its answer names no instance: it serves BRP without this crate's
    /// plugin, or its plugin is older than this server. `false` when the game was not asked
    /// successfully (it did not answer, or refused for another reason), which says nothing about
    /// it.
    pub answered_without_a_name: bool,
}

/// The answer of [`BrpClient::game_instance`], kept by the stores that name entities by id.
pub type GameName = std::result::Result<String, Unnamed>;

/// Whether the game that gave the answer `then` is certainly another process than the one that
/// gives `now`. Two different instances are. A game that answered without naming itself is also
/// another process than one that names itself later, because a process does not gain a plugin.
/// Anything else (a game that could not be asked, or two that cannot name themselves) is unknown.
#[must_use]
pub fn certainly_another_game(then: &GameName, now: &GameName) -> bool {
    match (then, now) {
        (Ok(then), Ok(now)) => then != now,
        (Err(then), Ok(_)) => then.answered_without_a_name,
        _ => false,
    }
}

/// BRP client with extensible command handler support.
///
/// Stateless over HTTP: "connected" means the last `rpc.discover` health
/// check returned a 2xx response.
pub struct BrpClient {
    config: Config,
    http: Client,
    url: String,
    connected: Arc<AtomicBool>,
    retry_count: u32,
    request_id: Arc<AtomicU64>,
    command_registry: Arc<CommandHandlerRegistry>,
    debug_router: Option<Arc<DebugCommandRouter>>,
}

impl std::fmt::Debug for BrpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrpClient")
            .field("config", &self.config)
            .field("url", &self.url)
            .field("connected", &self.is_connected())
            .field("retry_count", &self.retry_count)
            .field("has_debug_router", &self.debug_router.is_some())
            .finish()
    }
}

impl Clone for BrpClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            http: self.http.clone(),
            url: self.url.clone(),
            connected: self.connected.clone(),
            retry_count: self.retry_count,
            request_id: self.request_id.clone(),
            command_registry: self.command_registry.clone(),
            debug_router: self.debug_router.clone(),
        }
    }
}

impl BrpClient {
    pub fn new(config: &Config) -> Self {
        let command_registry = Arc::new(CommandHandlerRegistry::new());
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client builder with rustls must succeed");

        BrpClient {
            config: config.clone(),
            http,
            url: format!(
                "http://{}:{}",
                config.bevy_brp_host, config.bevy_brp_port
            ),
            connected: Arc::new(AtomicBool::new(false)),
            retry_count: 0,
            request_id: Arc::new(AtomicU64::new(1)),
            command_registry,
            debug_router: None,
        }
    }

    /// Initialize the client asynchronously with default handlers
    pub async fn init(&self) -> Result<()> {
        // Register core handler - safe async initialization
        let core_handler = Arc::new(CoreBrpHandler);
        self.command_registry.register(core_handler).await;
        Ok(())
    }

    /// Set the debug command router for handling debug commands
    pub fn with_debug_router(mut self, router: Arc<DebugCommandRouter>) -> Self {
        self.debug_router = Some(router);
        self
    }

    /// Register a custom command handler
    pub async fn register_handler(&self, handler: Arc<dyn BrpCommandHandler>) {
        self.command_registry.register(handler).await;
    }

    /// Get the command registry for external access
    pub fn command_registry(&self) -> Arc<CommandHandlerRegistry> {
        self.command_registry.clone()
    }

    pub async fn connect_with_retry(&mut self) -> Result<()> {
        const MAX_RETRIES: u32 = 5;
        const BASE_DELAY: Duration = Duration::from_millis(1000);

        while self.retry_count < MAX_RETRIES {
            match self.connect().await {
                Ok(()) => {
                    info!("Successfully connected to BRP at {}", self.url);
                    self.retry_count = 0;
                    return Ok(());
                }
                Err(e) => {
                    self.retry_count += 1;
                    let delay = BASE_DELAY * 2_u32.pow(self.retry_count.min(5));
                    warn!(
                        "Failed to connect to BRP (attempt {}/{}): {}. Retrying in {:?}",
                        self.retry_count, MAX_RETRIES, e, delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }

        // Start the next round from zero, so a later call tries again instead of returning at once.
        self.retry_count = 0;
        Err(Error::Connection(format!(
            "Failed to connect to BRP after {MAX_RETRIES} attempts"
        )))
    }

    /// HTTP "connect": probe the server with an `rpc.discover` request and
    /// accept any 2xx response as a healthy BRP endpoint.
    async fn connect(&self) -> Result<()> {
        debug!("Attempting BRP health check (rpc.discover) at {}", self.url);

        let discover = BrpRequest {
            method: builtin_methods::RPC_DISCOVER_METHOD.to_string(),
            id: Some(Value::from(self.request_id.fetch_add(1, Ordering::Relaxed))),
            params: None,
        };

        let response = self
            .http
            .post(&self.url)
            .json(&discover)
            .send()
            .await
            .map_err(|e| Error::Connection(format!("BRP health check failed: {e}")))?;

        if response.status().is_success() {
            self.connected.store(true, Ordering::Relaxed);
            Ok(())
        } else {
            self.connected.store(false, Ordering::Relaxed);
            Err(Error::Connection(format!(
                "BRP health check returned HTTP {}",
                response.status()
            )))
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Whether the game answers now. `is_connected` is the state the last probe left, refreshed by
    /// a heartbeat every 30 s, so a tool that read it alone refused with "not connected" for up to
    /// that long after a game came up. When the last known state is "no", probe once before saying so.
    pub async fn reachable(&self) -> bool {
        self.is_connected() || self.probe().await.is_ok()
    }

    /// The BRP endpoint this client talks to, for log messages.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// One health check with no retry and no sleep. It takes `&self` (the state it sets is atomic), so
    /// the background heartbeat calls it under the read lock and holds that lock for one HTTP request only.
    pub async fn probe(&self) -> Result<()> {
        let result = self.connect().await;
        if result.is_err() {
            self.connected.store(false, Ordering::Relaxed);
        }
        result
    }

    /// Send a request to the running game.
    ///
    /// **Nothing between the caller and the socket, and that is a fix.** This used to run every
    /// request through three throttles first: a rate limiter, a semaphore gated by a circuit
    /// breaker, and an adaptive sampler that could *silently discard* the request and return
    /// "Request skipped due to adaptive sampling" as an error. Any of the three could block, and one
    /// of them did — `guide` and `observe` both hung indefinitely through a real MCP client while a
    /// tool that touches no BRP answered instantly, and the health check kept
    /// succeeding every thirty seconds throughout. The transport was never the problem.
    ///
    /// None of it was earning its place. This is a localhost JSON-RPC call to a game the developer
    /// is looking at: rate-limiting it protects nothing, and a sampler that throws away a debugging
    /// request is the opposite of what a debugger is for. The permit was not even held across the
    /// request — `let _permit = ...` was dropped at the end of the enclosing `if let`, before the
    /// send — so the concurrency limit it appeared to impose never existed.
    pub async fn send_request(&mut self, request: &BrpRequest) -> Result<BrpResponse> {
        let start_time = Instant::now();
        let result = self.send_request_internal(request).await;
        debug!("BRP request completed in {:?}", start_time.elapsed());
        result
    }

    /// Internal send request without resource management.
    ///
    /// POSTs the JSON-RPC envelope and parses the `bevy_remote::BrpResponse`
    /// from the JSON-RPC response body.
    async fn send_request_internal(&mut self, request: &BrpRequest) -> Result<BrpResponse> {
        // Stamp a fresh JSON-RPC id unless the caller set one.
        let mut request = request.clone();
        if request.id.is_none() {
            request.id = Some(Value::from(self.request_id.fetch_add(1, Ordering::Relaxed)));
        }

        let response = self
            .http
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                self.connected.store(false, Ordering::Relaxed);
                if e.is_timeout() {
                    Error::Connection("Request timeout".to_string())
                } else {
                    Error::Connection(format!("BRP HTTP request failed: {e}"))
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            self.connected.store(false, Ordering::Relaxed);
            return Err(Error::Connection(format!(
                "BRP server returned HTTP {status}"
            )));
        }

        self.connected.store(true, Ordering::Relaxed);

        response.json::<BrpResponse>().await.map_err(Error::from)
    }

    /// Send several requests as one JSON-RPC batch. bevy_remote serves them one after another,
    /// each waiting for its answer, so they may land on different frames. Responses come back in the order of
    /// `requests`, matched by `id`; every request must carry one.
    pub async fn send_batch(&mut self, requests: &[BrpRequest]) -> Result<Vec<BrpResponse>> {
        let response = self.http.post(&self.url).json(requests).send().await.map_err(|e| {
            self.connected.store(false, Ordering::Relaxed);
            if e.is_timeout() {
                Error::Connection("Request timeout".to_string())
            } else {
                Error::Connection(format!("BRP HTTP request failed: {e}"))
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            self.connected.store(false, Ordering::Relaxed);
            return Err(Error::Connection(format!("BRP server returned HTTP {status}")));
        }
        self.connected.store(true, Ordering::Relaxed);
        let mut answers = response.json::<Vec<BrpResponse>>().await.map_err(Error::from)?;
        let mut ordered = Vec::with_capacity(requests.len());
        for request in requests {
            let Some(position) = answers.iter().position(|answer| answer.id == request.id) else {
                return Err(Error::Connection(format!(
                    "the game's batch reply has no answer for `{}`",
                    request.method
                )));
            };
            ordered.push(answers.swap_remove(position));
        }
        Ok(ordered)
    }

    /// Send a batched request: executes immediately like `send_request`.
    ///
    /// BRP is request/response over HTTP, so batching has no transport to
    /// amortize; this preserves the interface for existing callers.
    pub async fn send_batched_request(&mut self, request: BrpRequest) -> Result<BrpResponse> {
        self.send_request(&request).await
    }

    /// Which game process answers at this client's URL: the `instance` of `bevy_fairy/status`, new
    /// after a restart. State a server keeps about a game (an undo history, a diff baseline) names
    /// entities by id, and a restarted game gives the same ids to other entities; this is how that
    /// state tells its own game from another. The `Err` says why the game cannot say: it does not
    /// answer, it serves BRP without this crate's plugin, or its plugin is older than this server.
    pub async fn game_instance(&mut self) -> GameName {
        const METHOD: &str = "bevy_fairy/status";
        let request = BrpRequest { method: METHOD.to_string(), id: Some(Value::from(1)), params: None };
        instance_from(self.send_request(&request).await.map(|response| response.payload))
    }

    /// Disconnect: with HTTP there is no persistent connection to drop, so
    /// this simply marks the client disconnected.
    pub async fn disconnect(&mut self) {
        self.connected.store(false, Ordering::Relaxed);
        info!("Disconnected from BRP");
    }
}

/// Read the process out of the answer to `bevy_fairy/status`, or say why it cannot be read.
fn instance_from(answer: Result<BrpPayload>) -> GameName {
    const METHOD: &str = "bevy_fairy/status";
    match answer {
        Ok(BrpPayload::Result(status)) => {
            status.get("instance").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Unnamed {
                why: format!("`{METHOD}` names no `instance`: the game's plugin is older than this server"),
                answered_without_a_name: true,
            })
        }
        Ok(BrpPayload::Error(error)) => Err(Unnamed {
            why: refusal_text(METHOD, &error),
            answered_without_a_name: error.code == bevy_remote::error_codes::METHOD_NOT_FOUND,
        }),
        Err(e) => Err(Unnamed {
            why: format!("the game did not answer `{METHOD}`: {e}"),
            answered_without_a_name: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::brp::BrpError;
    use serde_json::json;

    fn refusal(code: i16) -> Result<BrpPayload> {
        Ok(BrpPayload::Error(BrpError { code, message: "no".to_string(), data: None }))
    }

    #[test]
    fn a_status_that_names_an_instance_names_the_game() {
        let answer = Ok(BrpPayload::Result(json!({"ready": true, "instance": "p1-n2-c3"})));
        assert_eq!(instance_from(answer), Ok("p1-n2-c3".to_string()));
    }

    /// These two say the game answered, and what it answered cannot be the plugin's of this
    /// version: no process gains a plugin, so a game that names itself later is another one.
    #[test]
    fn a_status_with_no_instance_or_no_such_method_is_a_game_that_answered_without_a_name() {
        let older = instance_from(Ok(BrpPayload::Result(json!({"ready": true, "frame": 4}))));
        assert!(older.expect_err("an older plugin names no instance").answered_without_a_name);
        let absent = instance_from(refusal(bevy_remote::error_codes::METHOD_NOT_FOUND));
        assert!(absent.expect_err("no plugin, no method").answered_without_a_name);
    }

    /// A game that was not asked successfully says nothing about which process it is.
    #[test]
    fn a_game_that_gave_no_usable_answer_is_not_known_to_have_no_plugin() {
        let internal = instance_from(refusal(bevy_remote::error_codes::INTERNAL_ERROR));
        assert!(!internal.expect_err("an internal error names nothing").answered_without_a_name);
        let down = instance_from(Err(Error::Connection("BRP server returned HTTP 500".to_string())));
        let down = down.expect_err("a game that is down names nothing");
        assert!(!down.answered_without_a_name);
        assert!(down.why.contains("did not answer"), "{}", down.why);
    }

    #[test]
    fn only_a_game_that_answered_without_a_name_is_another_process_than_one_that_names_itself() {
        let named = |id: &str| -> GameName { Ok(id.to_string()) };
        let nameless = |answered_without_a_name| -> GameName {
            Err(Unnamed { why: "no".to_string(), answered_without_a_name })
        };
        assert!(certainly_another_game(&named("a"), &named("b")));
        assert!(!certainly_another_game(&named("a"), &named("a")));
        assert!(certainly_another_game(&nameless(true), &named("a")));
        assert!(!certainly_another_game(&nameless(false), &named("a")));
        assert!(!certainly_another_game(&named("a"), &nameless(true)));
        assert!(!certainly_another_game(&nameless(true), &nameless(true)));
    }
}
