class BevyDebuggerMcp < Formula
  desc "AI-assisted debugging tools for Bevy games via Claude MCP"
  homepage "https://github.com/bevy-coven/bevy_fairy"
  url "https://github.com/bevy-coven/bevy_fairy/archive/v0.1.9.tar.gz"
  sha256 "PLACEHOLDER_SHA256"  # Update with actual SHA
  license "MIT"
  head "https://github.com/bevy-coven/bevy_fairy.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    
    # Install the setup script
    bin.install "setup-claude.sh" => "bevy-fairy-setup"
  end

  def post_install
    # Create symlinks for Claude Code compatibility
    claude_local_bin = File.expand_path("~/.local/bin")
    FileUtils.mkdir_p(claude_local_bin)
    
    binary_path = "#{HOMEBREW_PREFIX}/bin/bevy-fairy"
    symlink_path = "#{claude_local_bin}/bevy-fairy"
    
    if File.exist?(binary_path) && !File.exist?(symlink_path)
      File.symlink(binary_path, symlink_path)
    end
  end

  def caveats
    <<~EOS
      bevy-fairy has been installed!

      To complete setup for Claude Code/Desktop, run:
        bevy-fairy-setup

      This will:
      - Create necessary symlinks
      - Show you the configuration to add to Claude

      For manual configuration, add to:
      - Claude Code: ~/.claude/mcp_settings.json
      - Claude Desktop: ~/Library/Application Support/Claude/claude_desktop_config.json

      Example configuration:
        "bevy-fairy": {
          "command": "#{HOMEBREW_PREFIX}/bin/bevy-fairy",
          "args": ["--stdio"],
          "env": {
            "RUST_LOG": "info",
            "BEVY_BRP_HOST": "127.0.0.1",
            "BEVY_BRP_PORT": "15702"
          }
        }
    EOS
  end

  test do
    system "#{bin}/bevy-fairy", "--version"
  end
end