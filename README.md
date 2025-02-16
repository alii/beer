# Beer - Low Latency Audio Streamer

A cross-platform (Windows & macOS) desktop audio streaming application written in Rust, focusing on low latency.

## Features

- Capture system-wide audio (desktop audio)
- Stream audio over the network with minimal latency
- Auto-discovery of streaming servers on the local network
- Cross-platform support (Windows and macOS)

## Installation

### Pre-built Binaries

Download the latest release for your platform from the [Releases](../../releases) page.

### Building from Source

1. Install Rust (if you haven't already):

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Clone the repository:

   ```bash
   git clone https://github.com/alii/beer.git
   cd beer
   ```

3. Build in release mode:
   ```bash
   cargo build --release
   ```

The binary will be available in `target/release/audio_streamer_cli` (or `audio_streamer_cli.exe` on Windows).

## Usage

### Broadcasting Audio (Server)

To start broadcasting your desktop audio:

```bash
# Default settings (binds to 0.0.0.0:50001)
audio_streamer_cli broadcast

# Custom bind address
audio_streamer_cli broadcast -b "192.168.1.100:50001"
```

### Listening to Audio (Client)

To start receiving and playing audio:

```bash
# Auto-discover and connect to server
audio_streamer_cli listen

# Custom bind address
audio_streamer_cli listen -b "192.168.1.101:50001"
```

## Platform-Specific Notes

### Windows

- Uses WASAPI for system audio capture
- No additional setup required
- May need to allow the application through Windows Firewall on first run

### macOS

- Requires a virtual audio driver for system audio capture
- Recommended options:
  - [BlackHole](https://github.com/ExistentialAudio/BlackHole)
  - [Soundflower](https://github.com/mattingalls/Soundflower)

## Network Requirements

- UDP ports used:
  - 50000: Auto-discovery service
  - 50001: Audio streaming (default, configurable)
- Both the server and clients must be on the same local network
- Firewall must allow UDP traffic on the above ports

## Building

The project uses GitHub Actions to automatically build releases for both Windows and macOS. Each push to the master branch creates a new release with platform-specific binaries.

To build manually for a specific platform:

```bash
# For the current platform
cargo build --release

# For Windows (requires cross-compilation setup)
cargo build --release --target x86_64-pc-windows-msvc

# For macOS (requires macOS)
cargo build --release --target x86_64-apple-darwin
```

## License

MIT License - see [LICENSE](LICENSE) for details
