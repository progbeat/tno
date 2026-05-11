use std::collections::{BTreeMap, BTreeSet};
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use serde_json::Value;

use crate::types::{AgentConfig, EvaluatorError, TokenUsage};

pub(crate) struct AppServerRunner {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) messages: Receiver<Result<Value, String>>,
    pub(crate) reader: Option<JoinHandle<()>>,
    pub(crate) next_id: u64,
    pub(crate) token_usage_by_turn: BTreeMap<String, TokenUsage>,
}

pub(crate) struct LazyAppServerRunner {
    pub(crate) load_plugins: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) inner: Option<AppServerRunner>,
    pub(crate) sessions: BTreeSet<String>,
    pub(crate) retired_token_usage: TokenUsage,
}

impl LazyAppServerRunner {
    pub(crate) fn new(load_plugins: bool, agent: &AgentConfig) -> LazyAppServerRunner {
        LazyAppServerRunner {
            load_plugins,
            agent: agent.clone(),
            inner: None,
            sessions: BTreeSet::new(),
            retired_token_usage: TokenUsage::default(),
        }
    }

    pub(crate) fn inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(self.load_plugins, &self.agent)?);
        }
        match self.inner.as_mut() {
            Some(inner) => Ok(inner),
            None => Err("app-server runner is not initialized".into()),
        }
    }
}
