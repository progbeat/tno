use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::{Child, ChildStdin};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

use serde_json::Value;

use crate::config_types::AgentConfig;
use crate::evaluator_types::EvaluatorError;
use crate::thread_reuse_config::CarryoverTokenTarget;
use crate::token_usage_types::{
    ContextCompactionEvent, EvaluatorTurnUsage, TokenUsage, TokenUsageUpdate,
};

pub(crate) struct AppServerRunner {
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) messages: Receiver<Result<Value, String>>,
    pub(crate) reader: Option<JoinHandle<()>>,
    pub(crate) next_id: u64,
    pub(crate) token_usage_by_turn: BTreeMap<String, TokenUsage>,
    pub(crate) token_usage_updates_by_turn: BTreeMap<String, Vec<TokenUsageUpdate>>,
    pub(crate) context_compaction_events_by_turn: BTreeMap<String, Vec<ContextCompactionEvent>>,
    pub(crate) last_turn_usage: Option<EvaluatorTurnUsage>,
    pub(crate) carryover_token_target: CarryoverTokenTarget,
    pub(crate) turn_carryover_by_thread: BTreeMap<String, Vec<ThreadTurnCarryover>>,
    pub(crate) retired_sessions: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThreadTurnCarryover {
    pub(crate) turn_id: String,
    pub(crate) tokens: u64,
}

pub(crate) struct LazyAppServerRunner {
    pub(crate) root: PathBuf,
    pub(crate) load_plugins: bool,
    pub(crate) agent: AgentConfig,
    pub(crate) inner: Option<AppServerRunner>,
    pub(crate) sessions: BTreeSet<String>,
    pub(crate) retired_token_usage: TokenUsage,
}

impl LazyAppServerRunner {
    pub(crate) fn new(
        root: &std::path::Path,
        load_plugins: bool,
        agent: &AgentConfig,
    ) -> LazyAppServerRunner {
        LazyAppServerRunner {
            root: root.to_path_buf(),
            load_plugins,
            agent: agent.clone(),
            inner: None,
            sessions: BTreeSet::new(),
            retired_token_usage: TokenUsage::default(),
        }
    }

    pub(crate) fn inner(&mut self) -> Result<&mut AppServerRunner, EvaluatorError> {
        if self.inner.is_none() {
            self.inner = Some(AppServerRunner::new(
                &self.root,
                self.load_plugins,
                &self.agent,
            )?);
        }
        match self.inner.as_mut() {
            Some(inner) => Ok(inner),
            None => Err("app-server runner is not initialized".into()),
        }
    }
}

impl AppServerRunner {
    pub(crate) fn drain_retired_sessions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.retired_sessions)
            .into_iter()
            .collect()
    }
}
