use std::{collections::HashMap, sync::Arc};

use tokio::sync::Mutex;

use crate::managed_service::ManagedService;

#[derive(Clone)]
pub struct Instances {
    instances: HashMap<String, Arc<Mutex<Services>>>,
}

impl Instances {
    pub fn new() -> Self {
        Self {
            instances: Default::default(),
        }
    }

    pub fn insert(&mut self, id: String, services: Services) -> Option<Arc<Mutex<Services>>> {
        self.insert_raw(id, Arc::new(Mutex::new(services)))
    }

    pub fn insert_raw(
        &mut self,
        id: String,
        services: Arc<Mutex<Services>>,
    ) -> Option<Arc<Mutex<Services>>> {
        self.instances.insert(id, services)
    }

    pub fn remove(&mut self, id: &str) -> Option<Arc<Mutex<Services>>> {
        self.instances.remove(id)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Mutex<Services>>> {
        self.instances.get(id).map(|s| s.clone())
    }
}

type Services = HashMap<String, ManagedService>;
