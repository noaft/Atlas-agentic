//! Native journal ownership is explicitly carried into delegated workers.
use super::SessionJournal;
use crate::{
    agents::Agent,
    permissions::ToolEvent,
    runtime::context::{ContextPreparationRequest, PreparedContext, RuntimeContext},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
thread_local! {static TRACE:RefCell<Option<Trace>>=const{RefCell::new(None)};}
#[derive(Clone)]
pub(crate) struct Trace {
    journal: SessionJournal,
    turn: String,
    run: Option<String>,
    parent_run: Option<String>,
    fork_cursor: Option<super::ContextCursor>,
}
pub(crate) struct Guard {
    previous: Option<Trace>,
    _diagnostics: crate::diagnostics::SessionTraceGuard,
}
pub(crate) fn current() -> Option<Trace> {
    TRACE.with(|trace| trace.borrow().clone())
}
pub(crate) fn enter(trace: Trace) -> Guard {
    let diagnostics =
        crate::diagnostics::session_trace(&trace.journal.id().to_string(), &trace.turn);
    Guard {
        previous: TRACE.with(|current| current.replace(Some(trace))),
        _diagnostics: diagnostics,
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        TRACE.with(|trace| trace.replace(self.previous.take()));
    }
}
impl Trace {
    pub fn root(journal: SessionJournal, turn: &str) -> Self {
        Self {
            journal,
            turn: turn.into(),
            run: None,
            parent_run: None,
            fork_cursor: None,
        }
    }
    pub fn freeze(&self) -> Result<Self, String> {
        self.check()?;
        let mut trace = self.clone();
        if trace.fork_cursor.is_none() {
            trace.fork_cursor = Some(self.journal.metadata()?.context_cursor);
        }
        Ok(trace)
    }
    pub fn child(&self, run: &str) -> Self {
        Self {
            run: Some(run.into()),
            parent_run: self.run.clone(),
            fork_cursor: self
                .fork_cursor
                .clone()
                .or_else(|| self.journal.metadata().ok().map(|m| m.context_cursor)),
            ..self.clone()
        }
    }
    pub fn active(&self) -> bool {
        self.journal
            .metadata()
            .is_ok_and(|m| m.active_turn.as_deref() == Some(&self.turn))
    }
    pub fn check(&self) -> Result<(), String> {
        self.journal.check()?;
        if self.active() {
            Ok(())
        } else {
            Err("Session turn has ended".into())
        }
    }
    pub fn record(&self, kind: &str, mut data: Value) -> Result<(), String> {
        if kind == "child_run_finished" {
            self.journal.check()?;
        } else {
            self.check()?;
        }
        if let Some(run) = &self.run {
            data["agentRunId"] = json!(run);
        }
        if let Some(parent) = &self.parent_run {
            data["parentAgentRunId"] = json!(parent);
        }
        self.journal.record(&self.turn, kind, data)?;
        Ok(())
    }
    pub fn event(&self, event: &ToolEvent) -> Result<(), String> {
        self.check()?;
        self.journal
            .record_runtime_event_scoped(&self.turn, event, self.run.as_deref())
    }
}
pub(crate) struct WorkerContext<'a> {
    pub inner: &'a mut dyn RuntimeContext,
    pub trace: Option<&'a Trace>,
}
impl RuntimeContext for WorkerContext<'_> {
    fn check_cancelled(&self) -> Result<(), String> {
        if let Some(trace) = self.trace {
            trace.check()?;
        }
        self.inner.check_cancelled()
    }
    fn record_step(&self, kind: &str, data: Value) -> Result<(), String> {
        if let Some(trace) = self.trace {
            trace.record(kind, data)?;
        }
        Ok(())
    }
    fn prepare(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        self.prepare_with_control(agent, request, events, &|| false)
    }
    fn prepare_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<PreparedContext, String> {
        self.check_cancelled()?;
        let prepared = self.inner.prepare_with_control(
            agent,
            ContextPreparationRequest { ..request },
            events,
            &|| interrupted() || self.trace.is_some_and(|t| t.check().is_err()),
        )?;
        if let Some(trace) = self.trace {
            trace.record("child_context_prepared",json!({"source":"frozen_child_snapshot_and_current_continuation","parentCursor":trace.fork_cursor,"historyMessages":prepared.history.len(),"historyFingerprint":crate::conversations::memory_fingerprint(&prepared.history)?,"input":request.input,"inputSha256":format!("{:x}",Sha256::digest(request.input.as_bytes())),"model":agent.model_name(),"maxOutputTokens":prepared.max_output_tokens,"imageCount":request.images.len(),"toolNames":request.tools.iter().map(|t|&t.name).collect::<Vec<_>>()}))?;
        }
        Ok(prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::{LocalAgent, LocalAgentDefinition},
        models::{ModelProvider, ModelRequest},
        observability::ObservabilityConfig,
        runtime::context::StaticContext,
        sessions::{ContextCursor, SessionStore},
    };
    struct Model;
    impl ModelProvider for Model {
        fn complete(&self, _: &ModelRequest) -> Result<String, String> {
            unreachable!()
        }
    }
    #[test]
    fn frozen_child_context_retains_originating_parent_cursor() {
        let data = tempfile::tempdir().unwrap();
        let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
        let journal = store
            .open(None, None, chrono::Utc::now().fixed_offset())
            .unwrap();
        let turn = journal.start_turn("task").unwrap();
        journal
            .prepare_context(
                &turn,
                ContextCursor {
                    history_message_start: 2,
                    ..Default::default()
                },
                json!({}),
            )
            .unwrap();
        let child = Trace::root(journal.clone(), &turn).child("child-run");
        // Root continues while the child is queued or waiting for a tool.
        journal
            .prepare_context(
                &turn,
                ContextCursor {
                    history_message_start: 8,
                    ..Default::default()
                },
                json!({}),
            )
            .unwrap();
        let definition=LocalAgentDefinition::from_toml("name='fake'\ndisplay_name='Fake'\nmodel_provider='fake'\nmodel='fake'\nsystem_prompt='system'\nconversation_title_prompt='title'").unwrap();
        let agent = LocalAgent::new(definition, Box::new(Model));
        let mut inner = StaticContext {
            history: &[],
            activity_context: None,
        };
        let mut context = WorkerContext {
            inner: &mut inner,
            trace: Some(&child),
        };
        context
            .prepare(
                &agent,
                ContextPreparationRequest {
                    input: "child task",
                    tools: &[],
                    images: &[],
                    latest_usage: None,
                },
                &mut |_| {},
            )
            .unwrap();
        let events = journal.events().unwrap();
        assert_eq!(
            events.last().unwrap().data["parentCursor"]["historyMessageStart"],
            2
        );
        assert_eq!(
            journal
                .metadata()
                .unwrap()
                .context_cursor
                .history_message_start,
            8
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::{observability::ObservabilityConfig, sessions::SessionStore};
    #[test]
    fn child_completion_can_be_recorded_after_parent_failure_without_reopening_the_turn() {
        let data = tempfile::tempdir().unwrap();
        let store = SessionStore::new(data.path(), ObservabilityConfig::default().session).unwrap();
        let journal = store
            .open(None, None, chrono::Utc::now().fixed_offset())
            .unwrap();
        let turn = journal.start_turn("task").unwrap();
        let child = Trace::root(journal.clone(), &turn).child("run");
        child.record("child_run_started", json!({})).unwrap();
        journal.finish_turn(&turn, "failed").unwrap();
        assert!(child.check().is_err());
        child
            .record("child_run_finished", json!({"status":"cancelled"}))
            .unwrap();
        assert!(journal.metadata().unwrap().active_turn.is_none());
        assert_eq!(
            journal.events().unwrap().last().unwrap().kind,
            "child_run_finished"
        );
    }
}
