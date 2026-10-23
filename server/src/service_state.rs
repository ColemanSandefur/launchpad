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
