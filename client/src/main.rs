use interprocess::local_socket::{
    GenericNamespaced,
    tokio::{Stream, prelude::*},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    try_join,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let printname = "example.sock";
    let name = printname.to_ns_name::<GenericNamespaced>()?;

    let conn = Stream::connect(name).await?;

    let mut recver = BufReader::new(&conn);
    let mut sender = &conn;

    let mut buffer = String::with_capacity(128);

    let send = sender.write_all(b"Hello from client!\n");
    let recv = recver.read_line(&mut buffer);

    try_join!(send, recv)?;

    // Avoid holding up resources.
    drop(recver);
    drop(conn);

    // read_line keeps the line feed at the end.
    print!("Server answered: {buffer}");
    Ok(())
}
