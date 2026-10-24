#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// Service has not yet started or is waiting for dependencies.
    Pending,
    /// Process has been spawned, but health checks have not yet passed.
    Starting,
    /// Process is running and health checks are fully passing.
    Healthy,
    /// Process is running, but health checks are failing.
    Unhealthy,
    /// Service is intentionally shutting down.
    Stopping,
    /// Service process has exited successfully or was stopped.
    Stopped,
    /// Service process crashed or failed to start.
    Failed,
}

impl ServiceState {
    /// Returns true if the service is in a usable/ready state for dependents.
    pub fn is_available(&self) -> bool {
        matches!(self, ServiceState::Healthy)
    }

    pub fn is_dead(&self) -> bool {
        matches!(self, ServiceState::Stopped | ServiceState::Failed)
    }
}

impl From<ServiceState> for common::message::ServiceState {
    fn from(value: ServiceState) -> Self {
        match value {
            ServiceState::Pending => common::message::ServiceState::Pending,
            ServiceState::Starting => common::message::ServiceState::Starting,
            ServiceState::Healthy => common::message::ServiceState::Healthy,
            ServiceState::Unhealthy => common::message::ServiceState::Unhealthy,
            ServiceState::Stopping => common::message::ServiceState::Stopping,
            ServiceState::Stopped => common::message::ServiceState::Stopped,
            ServiceState::Failed => common::message::ServiceState::Failed,
        }
    }
}
