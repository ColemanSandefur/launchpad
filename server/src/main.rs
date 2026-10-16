use std::io;

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, prelude::*, tokio::Stream, traits::tokio::Listener,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    try_join,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let printname = "example.sock";
    let name = printname.to_ns_name::<GenericNamespaced>()?;

    let listener = match ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_tokio()
    {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            eprintln!(
                "Error: could not start server because the socket file is \
                occupied. Please check if {printname} is in use by another \
                process and try again."
            );
            return Err(e.into());
        }
        x => x?,
    };

    // This is a good place to inform clients that the server is ready.
    eprintln!("Server running at {printname}");

    loop {
        let conn = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("There was an error with an incoming connection: {e}");
                continue;
            }
        };

        tokio::spawn(async move {
            if let Err(e) = handle_conn(conn).await {
                eprintln!("Error while handling connection: {e}");
            }
        });
    }
}

async fn handle_conn(conn: Stream) -> io::Result<()> {
    let mut recver = BufReader::new(&conn);
    let mut sender = &conn;

    let mut buffer = String::with_capacity(128);

    let send = sender.write_all(b"Hello from server!\n");
    let recv = recver.read_line(&mut buffer);

    try_join!(recv, send)?;

    // Avoid holding up resources.
    drop(conn);

    // read_line keeps the line feed at the end.
    print!("Client answered: {buffer}");
    Ok(())
}
