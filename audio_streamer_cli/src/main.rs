use audio_streamer::{
    capture::{AudioCapture, DeviceType},
    network::{AudioReceiver, AudioSender},
    player::AudioPlayer,
};
use clap::{Parser, Subcommand};
use std::error::Error;
use std::io::{self, Write};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start capturing and broadcasting audio
    Broadcast {
        /// Optional address to bind to (default: "0.0.0.0:50001")
        #[arg(short, long)]
        bind: Option<String>,

        /// Skip device selection prompt and use default input device
        #[arg(short, long)]
        use_default: bool,
    },

    /// Start receiving and playing audio (auto-discovers server)
    Listen {
        /// Optional address to bind to (default: "0.0.0.0:50001")
        #[arg(short, long)]
        bind: Option<String>,
    },
}

fn select_input_device(capture: &AudioCapture) -> Result<usize, Box<dyn Error>> {
    let devices = capture.list_input_devices()?;

    println!("\nAvailable input devices:");
    println!("------------------------");
    for device in &devices {
        let device_type = match device.device_type {
            DeviceType::SystemAudio => "(System Audio)",
            DeviceType::Virtual => "(Virtual Device)",
            DeviceType::Physical => "(Physical Device)",
        };

        println!(
            "{}. {} {} {}",
            device.index + 1,
            device.name,
            if device.is_default { "(Default)" } else { "" },
            device_type
        );
    }

    println!("------------------------");

    print!("Select input device (1-{}): ", devices.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let selected = input
        .trim()
        .parse::<usize>()
        .map_err(|_| "Invalid input: please enter a number".to_string())?
        - 1;

    if selected >= devices.len() {
        return Err("Invalid device selection".into());
    }

    Ok(selected)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Broadcast { bind, use_default } => {
            println!("Starting audio capture...");
            let capture = AudioCapture::new()?;

            let (_tx, rx, _stream) = if use_default {
                capture.start_capture()?
            } else {
                let device_index = select_input_device(&capture)?;
                println!("Using selected input device... {}", device_index + 1);
                capture.start_capture_with_device(device_index)?
            };

            println!("Starting audio broadcaster...");
            println!("Clients can now connect automatically via the 'listen' command");
            let sender = AudioSender::new(bind.as_deref()).await?;
            sender.start_sending(rx).await?;
        }

        Commands::Listen { bind } => {
            println!("Starting audio receiver...");
            let receiver = AudioReceiver::new(bind.as_deref()).await?;
            println!("Listening on {}", receiver.local_addr()?);

            println!("Discovering audio server...");
            receiver.discover_server().await?;
            let server_addr = receiver.server_addr().await?;
            println!("Server found at {}! Starting playback...", server_addr);

            let player = AudioPlayer::new()?;
            let (tx, stream) = player.start_playback()?;

            println!("Audio playback started. Waiting for audio data...");
            println!("Press Ctrl+C to stop.");

            // Keep the stream alive and handle the receiving
            receiver.start_receiving(tx).await?;

            // Keep the stream variable to prevent it from being dropped
            drop(stream);
        }
    }

    Ok(())
}
