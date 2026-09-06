use clap::{Parser, Subcommand};
use aerowm_ipc::{IpcCommand, IpcResponse, WorkspaceAction, IPC_SOCKET_PATH};
use std::os::unix::net::UnixStream;
use std::io::{Read, Write};

#[derive(Parser)]
#[command(name = "aerowm-ctl", version, about = "CLI tool to control AeroWM")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Reloads the Luau configuration
    Reload,
    /// Workspace operations
    Workspace {
        #[arg(short, long)]
        next: bool,
        #[arg(short, long)]
        prev: bool,
        #[arg(short, long)]
        switch: Option<usize>,
    },
    /// Kill the focused window
    Kill,
    /// Exit the compositor
    Exit,
    /// Subscribe to live events (Pub/Sub)
    Subscribe,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let ipc_command = match cli.command {
        Commands::Reload => IpcCommand::ReloadConfig,
        Commands::Kill => IpcCommand::KillWindow,
        Commands::Exit => IpcCommand::Exit,
        Commands::Subscribe => IpcCommand::Subscribe,
        Commands::Workspace { next, prev, switch } => {
            let action = if next {
                WorkspaceAction::Next
            } else if prev {
                WorkspaceAction::Prev
            } else if let Some(id) = switch {
                WorkspaceAction::Switch(id)
            } else {
                eprintln!("Error: you must provide a workspace action");
                std::process::exit(1);
            };
            IpcCommand::Workspace { action }
        }
    };

    // Serialize command
    let payload = serde_json::to_string(&ipc_command)?;

    // Connect to IPC socket
    let stream_res = UnixStream::connect(IPC_SOCKET_PATH);
    
    let mut stream = match stream_res {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to AeroWM at {}. Is it running?", IPC_SOCKET_PATH);
            eprintln!("Error details: {}", e);
            std::process::exit(1);
        }
    };

    // Send payload
    stream.write_all(payload.as_bytes())?;
    stream.write_all(b"\n")?;

    if matches!(ipc_command, IpcCommand::Subscribe) {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stream);
        for line in reader.lines() {
            if let Ok(line) = line {
                println!("{}", line); // Print live events JSON
            } else {
                break;
            }
        }
        return Ok(());
    }

    // Read single response for other commands
    let mut response_buf = String::new();
    stream.read_to_string(&mut response_buf)?;

    if let Ok(response) = serde_json::from_str::<IpcResponse>(&response_buf) {
        if response.success {
            println!("Success");
            if let Some(msg) = response.message {
                println!("{}", msg);
            }
        } else {
            eprintln!("Error: {}", response.message.unwrap_or_else(|| "Unknown error".to_string()));
            std::process::exit(1);
        }
    } else {
        println!("Received raw response (unparseable): {}", response_buf);
    }

    Ok(())
}
