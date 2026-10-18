pub mod log;

use std::{collections::HashMap, io, process::Stdio, sync::Arc};

use common::{
    Channel,
    message::{ServerRequest, ServerResponse},
};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, prelude::*, tokio::Stream, traits::tokio::Listener,
};
use tokio::sync::Mutex;

use crate::log::LogBuffer;

struct ManagedService {
    child: tokio::process::Child, // Kept alive here so it doesn't drop/kill immediately
    log_buffer: LogBuffer,
}

type Services = HashMap<String, ManagedService>;

struct AppState {
    /// Maps service name -> Service state
    pub instances: Mutex<HashMap<String, Services>>,
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
        instances: Mutex::new(HashMap::new()),
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
                let mut instances = state.instances.lock().await;

                let Some(services) = instances.get_mut(&id) else {
                    chan.send(&ServerResponse::StatusResponse {
                        services: Default::default(),
                    })
                    .await
                    .unwrap();
                    continue;
                };

                let services = services
                    .iter_mut()
                    .map(|(name, service)| {
                        let status = match service.child.try_wait() {
                            Ok(None) => true,
                            Ok(Some(_)) => false,
                            Err(_) => false,
                        };

                        (name.clone(), status)
                    })
                    .collect();

                drop(instances);

                chan.send(&ServerResponse::StatusResponse { services })
                    .await?;
            }
            ServerRequest::Up { structure, cwd } => {
                let mut instances = state.instances.lock().await;

                let mut services: Services = HashMap::new();

                for (service_name, service) in structure.services {
                    let mut child = service
                        .to_command()
                        .current_dir(&cwd)
                        .kill_on_drop(true)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;

                    let stdout = child.stdout.take().expect("Failed to take stdout");
                    let stderr = child.stderr.take().expect("Failed to take stderr");

                    let log_buffer = LogBuffer::new(1000);

                    let buffer_clone = log_buffer.clone();
                    tokio::spawn(async move {
                        let reader = tokio::io::BufReader::new(stdout);
                        let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
                        while let Ok(Some(line)) = lines.next_line().await {
                            buffer_clone.push(line).await;
                        }
                    });

                    let buffer_clone = log_buffer.clone();
                    tokio::spawn(async move {
                        let reader = tokio::io::BufReader::new(stderr);
                        let mut lines = tokio::io::AsyncBufReadExt::lines(reader);
                        while let Ok(Some(line)) = lines.next_line().await {
                            buffer_clone.push(line).await;
                        }
                    });

                    services.insert(service_name, ManagedService { child, log_buffer });
                }

                instances.insert(id.clone(), services);

                drop(instances);

                chan.send(&ServerResponse::Initialized).await?;
            }
            ServerRequest::Down => {
                let mut instances = state.instances.lock().await;

                let Some(services) = instances.get_mut(&id) else {
                    chan.send(&ServerResponse::Initialized).await.unwrap();
                    continue;
                };

                for (service_name, service) in services {
                    println!("Killing service {id} - {service_name}");
                    service.child.start_kill().unwrap();
                }

                chan.send(&ServerResponse::Initialized).await?;
            }
            ServerRequest::ServiceLog {
                service_name,
                start_idx,
            } => {
                let instances = state.instances.lock().await;

                let Some(services) = instances.get(&id) else {
                    chan.send(&ServerResponse::ServiceLogResponse {
                        service_name,
                        lines: vec![],
                        next_idx: 0,
                        dropped_history: false,
                    })
                    .await
                    .unwrap();
                    continue;
                };

                match services.get(&service_name) {
                    Some(service) => {
                        let result = service
                            .log_buffer
                            .get_since(start_idx.unwrap_or_default())
                            .await;

                        chan.send(&ServerResponse::ServiceLogResponse {
                            service_name,
                            lines: result.lines,
                            next_idx: result.next_idx,
                            dropped_history: result.dropped_history,
                        })
                        .await
                        .unwrap();
                    }
                    None => {
                        chan.send(&ServerResponse::ServiceLogResponse {
                            service_name,
                            lines: vec![],
                            next_idx: 0,
                            dropped_history: false,
                        })
                        .await
                        .unwrap();
                    }
                };
            }
        }
    }

    Ok(())
}
