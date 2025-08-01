use std::{borrow::Cow, sync::Arc};

use charon_lib::event::{DomainEvent, Event, Mode, Topic};
use tokio::{
    sync::{
        RwLock,
        mpsc::{self, Sender},
    },
    task::JoinHandle,
};
use tracing::{debug, error, info};

use crate::{
    actor::{KeyScanner, Pipeline},
    broker::EventBroker,
    config::CharonConfig,
    domain::{
        ActorState, ProcessorState,
        traits::{Actor, Processor},
    },
};

type ProcessorCtor = fn(ProcessorState) -> Box<dyn Processor + Send + Sync>;

pub struct Daemon {
    tasks: Vec<JoinHandle<()>>,
    broker: EventBroker,
    event_tx: Sender<Event>,
    mode: Arc<RwLock<Mode>>,
    config: CharonConfig,
}

impl Daemon {
    pub fn new() -> Self {
        let (event_tx, broker_rx) = mpsc::channel::<Event>(128);
        Self {
            tasks: Vec::new(),
            broker: EventBroker::new(broker_rx),
            event_tx,
            mode: Arc::new(RwLock::new(Mode::PassThrough)),
            config: CharonConfig::default(),
        }
    }

    pub async fn run(&mut self) {
        info!("Charon is ready...");
        self.broker.run().await;
        self.stop().await;
    }

    pub async fn stop(&mut self) {
        let event = Event::new("broker".into(), DomainEvent::Exit);
        self.broker.broadcast(&event, true).await;
    }

    pub async fn shutdown(&mut self) {
        for handle in self.tasks.drain(..) {
            if let Err(err) = handle.await {
                error!("Error while sutting down an actor: {err}");
            }
        }
    }

    fn register_actor<T: Actor>(
        &mut self,
        name: Cow<'static, str>,
        init: T::Init,
        topics: &'static [Topic],
        config: CharonConfig,
        processors: Vec<Box<dyn Processor + Send + Sync>>,
    ) -> &mut Self {
        let (pt_tx, pt_rx) = mpsc::channel::<Event>(128);
        self.broker.add_subscriber(pt_tx, name.clone(), topics);
        let state = ActorState::new(
            name.clone(),
            self.mode.clone(),
            self.event_tx.clone(),
            pt_rx,
            config,
            processors,
        );
        match T::spawn(state, init) {
            Ok(task) => self.tasks.push(task),
            Err(err) => error!("Couldn't spawn an actor {name} due to error: {err}"),
        }
        self
    }

    pub fn add_actor<T: Actor<Init = ()>>(&mut self, topics: &'static [Topic]) -> &mut Self {
        self.register_actor::<T>(
            T::name().into(),
            (),
            topics,
            self.config.clone(),
            Vec::new(),
        )
    }

    pub fn add_actor_conditionally<T: Actor<Init = ()>>(
        &mut self,
        should_add: bool,
        topics: &'static [Topic],
    ) -> &mut Self {
        if should_add {
            self.add_actor::<T>(topics);
        }
        self
    }

    pub fn add_scanners(&mut self, topics: &'static [Topic]) -> &mut Self {
        for (name, config) in self.config.get_config_per_keyboard() {
            debug!("Registering scanner: {name}");
            self.register_actor::<KeyScanner>(
                format!("KeyScanner-{name}").into(),
                name,
                topics,
                config,
                Vec::new(),
            );
        }
        self
    }

    pub fn add_actor_with_init<T: Actor>(
        &mut self,
        init: T::Init,
        topics: &'static [Topic],
    ) -> &mut Self {
        self.register_actor::<T>(
            T::name().into(),
            init,
            topics,
            self.config.clone(),
            Vec::new(),
        )
    }

    pub fn add_actor_with_processors<T: Actor<Init = ()>>(
        &mut self,
        topics: &'static [Topic],
        factories: &[ProcessorCtor],
    ) -> &mut Self {
        let state = ProcessorState::new(T::name().into(), self.mode.clone(), self.config.clone());
        let processors: Vec<_> = factories.iter().map(|f| f(state.clone())).collect();
        self.register_actor::<T>(
            T::name().into(),
            (),
            topics,
            self.config.clone(),
            processors,
        )
    }

    pub fn add_pipeline(
        &mut self,
        name: &'static str,
        topics: &'static [Topic],
        factories: &[ProcessorCtor],
    ) -> &mut Self {
        let state = ProcessorState::new(name.into(), self.mode.clone(), self.config.clone());
        let processors: Vec<_> = factories.iter().map(|f| f(state.clone())).collect();
        self.register_actor::<Pipeline>(name.into(), (), topics, self.config.clone(), processors);
        self
    }

    pub fn update_config(&mut self, transform_cfg: fn(&mut CharonConfig)) -> &mut Self {
        (transform_cfg)(&mut self.config);
        self
    }

    pub fn with_config(&mut self, config: CharonConfig) -> &mut Self {
        self.config = config;
        self
    }
}
