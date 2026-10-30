class BevyDebuggerMcp < Formula
  desc "MCP server for debugging Bevy games with Claude"
  homepage "https://github.com/ladvien/bevy_fairy"
  version "0.1.0"
  license "GPL-3.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/ladvien/bevy_fairy/releases/download/v#{version}/bevy-fairy-aarch64-apple-darwin.tar.gz"
      sha256 "YOUR_SHA256_HERE"
    else
      url "https://github.com/ladvien/bevy_fairy/releases/download/v#{version}/bevy-fairy-x86_64-apple-darwin.tar.gz"
      sha256 "YOUR_SHA256_HERE"
    end
  end

  on_linux do
    url "https://github.com/ladvien/bevy_fairy/releases/download/v#{version}/bevy-fairy-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "YOUR_SHA256_HERE"
  end

  def install
    bin.install "bevy-fairy"
  end

  def post_install
    # Create config directory
    config_dir = Pathname.new(Dir.home) / ".config" / "bevy-fairy"
    config_dir.mkpath

    # Create default config if it doesn't exist
    config_file = config_dir / "config.toml"
    unless config_file.exist?
      config_file.write <<~EOS
        [connection]
        bevy_host = "localhost"
        bevy_port = 15702
        mcp_port = 3000

        [logging]
        level = "info"
        debug_mode = false

        [features]
        auto_reconnect = true
        checkpoint_interval = 1000
        max_recording_size = "100MB"
      EOS
    end
  end

  def caveats
    <<~EOS
      ✨ bevy-fairy has been installed!

      bevy-fairy is an MCP server over stdio: add `bevy-fairy` to your MCP
      client's server list and the client starts it. There is nothing to run
      as a service.

      Configuration file is located at:
        ~/.config/bevy-fairy/config.toml

      Add bevy_fairy's DebuggerPlugin (feature `plugin`) to your Bevy game.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/bevy-fairy --version")
    assert_match "--version", shell_output("#{bin}/bevy-fairy --help")
  end
end