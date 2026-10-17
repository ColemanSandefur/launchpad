use std::{collections::HashMap, io, sync::Arc};

use common::{
    Channel,
    message::{ServerRequest, ServerResponse},
};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, prelude::*, tokio::Stream, traits::tokio::Listener,
};
use tokio::sync::Mutex;

struct AppState {
    pub instance_state: Mutex<HashMap<String, bool>>,
}

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

    let state: Arc<AppState> = Arc::new(AppState {
        instance_state: Mutex::new(Default::default()),
    });

    loop {
        let conn = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("There was an error with an incoming connection: {e}");
                continue;
            }
        };

        let app_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(app_state, conn).await {
                eprintln!("Error while handling connection: {e}");
            }
        });
    }
}

async fn handle_conn(state: Arc<AppState>, conn: Stream) -> Result<(), Box<dyn std::error::Error>> {
    let mut chan = Channel::from_stream(conn);

    let ServerRequest::Initial { id } = chan.receive().await.unwrap().unwrap() else {
        panic!();
    };

    println!("Bound connection to id: {id}");

    chan.send(&ServerResponse::Initialized).await.unwrap();

    while let Some(message) = chan.receive::<ServerRequest>().await {
        let message = match message {
            Ok(msg) => msg,
            Err(err) => {
                println!("An unexpected error occurred: {err:?}");
                continue;
            }
        };

        // 7. Handle the message logic
        match message {
            ServerRequest::Initial { .. } => {}
            ServerRequest::Status => {
                let lock = state.instance_state.lock().await;
                let state = lock.get(&id).cloned().unwrap_or(false);
                drop(lock);

                let message = ServerResponse::StatusResponse { up: state };
                chan.send(&message).await?;
            }
        }
    }

    Ok(())
}
