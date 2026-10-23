mod dependency_list;
pub mod instances;
pub mod log;
pub mod managed_service;
mod process;
mod service_state;

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use anyhow::Result;
use common::{
    Channel,
    message::{ServerRequest, ServerResponse, ServiceLogResponse, StatusResponse},
};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, prelude::*, tokio::Stream, traits::tokio::Listener,
};
use tokio::{
    runtime::Handle,
    sync::{Mutex, watch},
};

use crate::{
    dependency_list::DependencyList, instances::Instances, managed_service::ManagedService,
    service_state::ServiceState,
};

type Services = HashMap<String, ManagedService>;

struct AppState {
    /// Maps service name -> Service state
    pub instances: Mutex<Instances>,
}

impl AppState {
    pub async fn get_instance(&self, id: &str) -> Option<Arc<Mutex<Services>>> {
        self.instances.lock().await.get(id)
    }
    pub async fn remove_instance(&self, id: &str) -> Option<Arc<Mutex<Services>>> {
        self.instances.lock().await.remove(id)
    }
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
        instances: Mutex::new(Instances::new()),
    });

    tokio::spawn(async {
        let metrics = Handle::current().metrics();

        loop {
            println!("{} threads active", metrics.num_alive_tasks());

            tokio::time::sleep(Duration::from_secs(10)).await;
        }
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
        handle_event(&state, &mut chan, &id, message).await?;
    }

    Ok(())
}

async fn handle_event(
    state: &AppState,
    chan: &mut Channel,
    id: &String,
    message: ServerRequest,
) -> anyhow::Result<()> {
    match message {
        ServerRequest::Initial { .. } => {}
        ServerRequest::Ping => {
            chan.send(&ServerResponse::Acknowledge).await?;
        }
        ServerRequest::Status => {
            let Some(services) = state.get_instance(&id).await else {
                chan.send(&ServerResponse::StatusResponse(StatusResponse {
                    services: Default::default(),
                }))
                .await
                .unwrap();
                return Ok(());
            };

            let statuses = services
                .lock()
                .await
                .iter()
                .map(|(name, service)| {
                    (
                        name.clone(),
                        common::message::ServiceStatus {
                            state: service.status().into(),
                        },
                    )
                })
                .collect::<HashMap<_, _>>();

            chan.send(&ServerResponse::StatusResponse(StatusResponse {
                services: statuses,
            }))
            .await?;
        }
        ServerRequest::Up { structure, cwd } => {
            let mut channels = HashMap::new();

            for service_name in structure.services.keys() {
                channels.insert(service_name.clone(), watch::channel(ServiceState::Pending));
            }

            let cwd = Arc::new(cwd);
            let mut services: Services = HashMap::new();

            for (service_name, service) in structure.services {
                let Some((health_tx, health_rx)) = channels.get(&service_name).cloned() else {
                    continue;
                };

                // Collect the receivers for this service's 'after' list
                let dependencies: DependencyList = service
                    .after
                    .iter()
                    .filter_map(|dep_name| channels.get(dep_name).map(|(_, rx)| rx.clone()))
                    .collect::<Vec<_>>()
                    .into();

                let mut managed_service =
                    ManagedService::new(service, dependencies, cwd.clone(), health_tx, health_rx);

                let _ = managed_service.start();

                services.insert(service_name.clone(), managed_service);
            }

            state.instances.lock().await.insert(id.clone(), services);
            chan.send(&ServerResponse::Acknowledge).await?;
        }
        ServerRequest::Down => {
            state.remove_instance(&id).await;

            chan.send(&ServerResponse::Acknowledge).await?;
        }
        ServerRequest::ServiceLog {
            service_name,
            start_idx,
        } => {
            let Some(services_mutex) = state.get_instance(&id).await else {
                chan.send(&ServerResponse::ServiceLogResponse(ServiceLogResponse {
                    service_name,
                    lines: vec![],
                    next_idx: 0,
                    dropped_history: false,
                }))
                .await
                .unwrap();
                return Ok(());
            };

            let services = services_mutex.lock().await;

            match services.get(&service_name) {
                Some(service) => {
                    let result = service.log().get_since(start_idx.unwrap_or_default()).await;

                    chan.send(&ServerResponse::ServiceLogResponse(ServiceLogResponse {
                        service_name,
                        lines: result.lines,
                        next_idx: result.next_idx,
                        dropped_history: result.dropped_history,
                    }))
                    .await
                    .unwrap();
                }
                None => {
                    chan.send(&ServerResponse::ServiceLogResponse(ServiceLogResponse {
                        service_name,
                        lines: vec![],
                        next_idx: 0,
                        dropped_history: false,
                    }))
                    .await
                    .unwrap();
                }
            };
        }
        ServerRequest::Start { service_name } => {
            let Some(services_mutex) = state.get_instance(id).await else {
                chan.send(&ServerResponse::Failure(
                    "Unable to get the instance".into(),
                ))
                .await?;
                return Ok(());
            };

            let mut services = services_mutex.lock().await;

            let response = match services.get_mut(&service_name) {
                Some(service) => match service.status() {
                    x if x.is_dead() => match service.start() {
                        Ok(_) => ServerResponse::Acknowledge,
                        Err(err) => ServerResponse::Failure(format!("{err:?}")),
                    },
                    _ => ServerResponse::Acknowledge,
                },
                None => ServerResponse::Failure("No service found with that name".into()),
            };

            chan.send(&response).await?;
        }
        ServerRequest::Stop { service_name } => {
            let Some(services_mutex) = state.get_instance(id).await else {
                chan.send(&ServerResponse::Failure(
                    "Unable to get the instance".into(),
                ))
                .await?;
                return Ok(());
            };

            let mut services = services_mutex.lock().await;

            let response = match services.get_mut(&service_name) {
                Some(service) => {
                    service.stop().await;
                    ServerResponse::Acknowledge
                }
                None => ServerResponse::Failure("No service found with that name".into()),
            };

            chan.send(&response).await?;
        }
    }

    Ok(())
}
