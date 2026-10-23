use tokio::sync::watch;

use crate::service_state::ServiceState;

pub struct DependencyList(pub Vec<watch::Receiver<ServiceState>>);

impl DependencyList {
    pub async fn wait_for_healthy(&mut self) {
        for rx in self.0.iter_mut() {
            if !rx.borrow().is_available() {
                while rx.changed().await.is_ok() {
                    if rx.borrow().is_available() {
                        break;
                    }
                }
            }
        }
    }
}

impl From<Vec<watch::Receiver<ServiceState>>> for DependencyList {
    fn from(value: Vec<watch::Receiver<ServiceState>>) -> Self {
        Self(value)
    }
}
