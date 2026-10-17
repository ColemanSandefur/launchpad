use clap::{Parser, Subcommand};
use common::{
    Channel,
    message::{ServerRequest, ServerResponse},
};
use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream, prelude::*},
};

#[derive(Parser, Debug)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Up,
    Down,
    Status,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let printname = "example.sock";
    let name = printname.to_ns_name::<GenericNamespaced>()?;

    let conn = Stream::connect(name).await?;

    let mut chan = Channel::from_stream(conn);

    let message = ServerRequest::Initial {
        id: "Hello!".to_string(),
    };

    chan.send(&message).await.unwrap();

    let msg = chan.receive::<ServerResponse>().await.unwrap().unwrap();

    let ServerResponse::Initialized = msg else {
        panic!();
    };

    match args.command {
        Commands::Up => todo!(),
        Commands::Down => todo!(),
        Commands::Status => {
            chan.send(&ServerRequest::Status).await?;

            let msg = chan.receive::<ServerResponse>().await.unwrap().unwrap();

            let ServerResponse::StatusResponse { up } = msg else {
                panic!();
            };

            let status = match up {
                true => "UP",
                false => "DOWN",
            };

            println!("Status: {}", status)
        }
    }

    // read_line keeps the line feed at the end.
    print!("Server answered: {:?}", msg);
    Ok(())
}
