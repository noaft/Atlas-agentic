use crate::{
    agents::{Agent, AgentRequest},
    conversations::*,
    media::ImageData,
    models::ModelUsage,
    permissions::ToolEvent,
    tools::ToolDefinition,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextStatus {
    pub available: bool,
    pub message: String,
    pub estimated: bool,
    pub input_tokens: Option<u64>,
    pub max_tokens: Option<u64>,
}
#[derive(Clone)]
pub struct PreparedContext {
    pub memory_blocks: Vec<MemoryBlock>,
    pub history: Vec<ConversationMessage>,
    pub activity_context: String,
    pub max_output_tokens: Option<u64>,
}
pub struct ContextPreparationRequest<'a> {
    pub input: &'a str,
    pub tools: &'a [ToolDefinition],
    pub images: &'a [ImageData],
    pub latest_usage: Option<&'a ModelUsage>,
}
pub trait RuntimeContext {
    fn check_cancelled(&self) -> Result<(), String> {
        Ok(())
    }

    fn record_step(&self, _kind: &str, _data: serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn prepare_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<PreparedContext, String> {
        if interrupted() {
            return Err("Context preparation interrupted".into());
        }
        self.prepare(agent, request, events)
    }

    fn prepare(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String>;
}
pub struct StaticContext<'a> {
    pub history: &'a [ConversationMessage],
    pub activity_context: Option<&'a str>,
}

pub struct CancellableContext<'a> {
    pub inner: &'a mut dyn RuntimeContext,
    pub cancelled: Option<&'a std::sync::atomic::AtomicBool>,
}
impl RuntimeContext for CancellableContext<'_> {
    fn check_cancelled(&self) -> Result<(), String> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Err("Request cancelled".into());
        }
        self.inner.check_cancelled()
    }
    fn record_step(&self, kind: &str, data: serde_json::Value) -> Result<(), String> {
        self.inner.record_step(kind, data)
    }
    fn prepare_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<PreparedContext, String> {
        self.check_cancelled()?;
        let cancelled = self.cancelled;
        let prepared = self
            .inner
            .prepare_with_control(agent, request, events, &|| {
                interrupted()
                    || cancelled.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            })?;
        self.check_cancelled()?;
        Ok(prepared)
    }
    fn prepare(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        self.check_cancelled()?;
        let prepared = self.inner.prepare(agent, request, events)?;
        self.check_cancelled()?;
        Ok(prepared)
    }
}
impl RuntimeContext for StaticContext<'_> {
    fn prepare_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<PreparedContext, String> {
        let prepared = self.prepare(agent, ContextPreparationRequest { ..request }, events)?;
        preflight_snapshot(agent, &request, prepared, events, interrupted)
    }
    fn prepare(
        &mut self,
        _: &dyn Agent,
        _: ContextPreparationRequest<'_>,
        _: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        Ok(PreparedContext {
            memory_blocks: vec![],
            history: self.history.to_vec(),
            activity_context: self.activity_context.unwrap_or_default().into(),
            max_output_tokens: None,
        })
    }
}
fn preflight_snapshot(
    agent: &dyn Agent,
    request: &ContextPreparationRequest<'_>,
    mut prepared: PreparedContext,
    events: &mut dyn FnMut(&ToolEvent),
    interrupted: &dyn Fn() -> bool,
) -> Result<PreparedContext, String> {
    let limits = agent.model_limits()?;
    if let Some(limits) = limits {
        limits.validate()?;
        prepared.max_output_tokens = Some(
            agent
                .memory_policy()
                .cloned()
                .unwrap_or_default()
                .output_reserve_tokens
                .min(limits.max_output_tokens),
        );
    }
    let estimate =
        agent.estimate_request_with_control(&agent_request(&prepared, request), interrupted)?;
    if let Some(limits) = limits {
        let reserve = prepared.max_output_tokens.unwrap_or_default();
        if estimate.is_some_and(|e| e.tokens.saturating_add(reserve) >= limits.context_window) {
            return Err(
                "Request exceeds model context; reduce the input or choose a larger model".into(),
            );
        }
    }
    if estimate.is_some() {
        let status = ContextStatus {
            available: limits.is_some(),
            message: if limits.is_some() {
                "Context checked before inference; automatic compaction unavailable in bounded history mode"
            } else {
                "Model capacity unknown; configure model limits to check context fit"
            }.into(),
            estimated: estimate.is_some_and(|e| e.estimated),
            input_tokens: estimate.map(|e| e.tokens),
            max_tokens: limits.map(|l| l.context_window),
        };
        events(&ToolEvent::ContextStatus { status });
    }
    Ok(prepared)
}

pub(crate) struct SnapshotContext(pub PreparedContext);
impl RuntimeContext for SnapshotContext {
    fn prepare(
        &mut self,
        _: &dyn Agent,
        _: ContextPreparationRequest<'_>,
        _: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        Ok(self.0.clone())
    }
}

pub struct ConversationContext<'a> {
    pub store: &'a ConversationStore,
    pub conversation: Option<Conversation>,
    pub policy: ContextPolicy,
    pub cancelled: Option<&'a std::sync::atomic::AtomicBool>,
}
impl ConversationContext<'_> {
    pub(crate) fn prepared(&self) -> PreparedContext {
        let mut result = PreparedContext {
            memory_blocks: vec![],
            history: vec![],
            activity_context: String::new(),
            max_output_tokens: None,
        };
        if let Some(conversation) = &self.conversation {
            let start = conversation
                .memory
                .as_ref()
                .map_or(0, |m| m.compacted_message_count);
            result.memory_blocks = conversation
                .memory
                .as_ref()
                .map(|m| m.blocks.clone())
                .unwrap_or_default();
            result.history = conversation.messages[start..].to_vec();
            annotate_interrupted_history(&mut result.history, start, &conversation.activities);
            let activities = conversation
                .activities
                .iter()
                .filter(|a| a.assistant_message_index >= start)
                .cloned()
                .collect::<Vec<_>>();
            result.activity_context = render_activity_context(&activities);
        }
        result
    }
    pub fn compact(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        trigger: CompactionTrigger,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<CompactionResult, String> {
        self.compact_with_control(agent, request, trigger, events, &|| false)
    }
    fn compact_with_control(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        trigger: CompactionTrigger,
        events: &mut dyn FnMut(&ToolEvent),
        interrupted: &dyn Fn() -> bool,
    ) -> Result<CompactionResult, String> {
        self.check_cancelled()?;
        let cancelled = self.cancelled;
        let interrupted = &|| {
            interrupted() || cancelled.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        };
        if let Some(conversation) = &self.conversation {
            let boundary = retained_start(&conversation.messages, self.policy.keep_recent_messages);
            let covered = conversation
                .memory
                .as_ref()
                .map_or(0, |memory| memory.compacted_message_count);
            if boundary <= covered && conversation.memory.is_none() {
                return Ok(CompactionResult {
                    memory: conversation.memory.clone(),
                    message: "No older messages to compact; recent messages preserved".into(),
                });
            }
        }
        let limits = agent
            .model_limits()?
            .ok_or("Model capacity unknown; configure model limits before compacting")?;
        limits.validate()?;
        let prepared = self.prepared();
        let estimate = agent
            .estimate_request_with_control(&agent_request(&prepared, &request), interrupted)?
            .ok_or(
                "Token budgeting unavailable for this input; compaction cannot be checked safely",
            )?;
        let Some(conversation) = &self.conversation else {
            return Ok(CompactionResult {
                memory: None,
                message: "No saved conversation to compact".into(),
            });
        };
        let old_id = conversation
            .memory
            .as_ref()
            .map(|m| m.checkpoint_id.clone());
        let boundary = retained_start(&conversation.messages, self.policy.keep_recent_messages);
        let wanted = choose_block_count(
            trigger,
            estimate.tokens as f64 / limits.context_window as f64 * 100.0,
        );
        let mut protected = self.prepared();
        protected.history = conversation.messages[boundary..].to_vec();
        annotate_interrupted_history(&mut protected.history, boundary, &conversation.activities);
        protected.activity_context = render_activity_context(
            &conversation
                .activities
                .iter()
                .filter(|a| a.assistant_message_index >= boundary)
                .cloned()
                .collect::<Vec<_>>(),
        );
        // Probe the message wrappers too, using maximum source indices for conservative metadata overhead.
        protected.memory_blocks = (0..wanted)
            .map(|_| MemoryBlock {
                start_index: boundary,
                end_index_exclusive: boundary,
                content: String::new(),
            })
            .collect();
        let protected_tokens = agent
            .estimate_request_with_control(&agent_request(&protected, &request), interrupted)?
            .ok_or("Protected context token budget unavailable")?
            .tokens;
        let reserve = self
            .policy
            .output_reserve_tokens
            .min(limits.max_output_tokens);
        let threshold = limits
            .context_window
            .saturating_mul(self.policy.auto_trigger_percent)
            / 100;
        let output_budget = threshold
            .saturating_sub(protected_tokens.saturating_add(reserve).saturating_add(1))
            / wanted as u64;
        if output_budget == 0 {
            return Err(
                "Protected context leaves no space for memory summaries; original memory retained"
                    .into(),
            );
        }
        let result = super::super::conversations::compaction::compact(
            super::super::conversations::compaction::CompactionInput {
                conversation,
                output_budget,
                agent,
                policy: &self.policy,
                cancelled: self.cancelled,
                interrupted,
                trigger,
                occupancy: estimate.tokens as f64 / limits.context_window as f64 * 100.0,
                estimated: estimate.estimated,
                limits,
            },
            &mut |progress| events(&ToolEvent::Compaction { progress }),
        )?;
        let Some(memory) = &result.memory else {
            return Ok(result);
        };
        if Some(&memory.checkpoint_id) == old_id.as_ref() {
            return Ok(result);
        }
        let id = conversation.metadata.id.clone();
        let mut candidate = conversation.clone();
        candidate.memory = Some(memory.clone());
        let previous = self.conversation.replace(candidate);
        let next = self.prepared();
        let fit = agent.estimate_request_with_control(&agent_request(&next, &request), interrupted);
        let reserved = self
            .policy
            .output_reserve_tokens
            .min(limits.max_output_tokens);
        if !matches!(fit,Ok(Some(n)) if n.tokens.saturating_add(reserved)<limits.context_window.saturating_mul(self.policy.auto_trigger_percent)/100)
        {
            self.conversation = previous;
            return Err("Compacted memory plus protected recent messages/current input cannot fit below threshold; original memory retained".into());
        }
        if self
            .cancelled
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            self.conversation = previous;
            return Err("Compaction cancelled; original memory retained".into());
        }
        if let Err(e) = self.store.replace_memory(&id, old_id.as_deref(), memory) {
            self.conversation = previous;
            return Err(e);
        }
        Ok(result)
    }
}
fn agent_request<'a>(
    prepared: &'a PreparedContext,
    request: &'a ContextPreparationRequest<'a>,
) -> AgentRequest<'a> {
    AgentRequest {
        input: request.input,
        history: &prepared.history,
        memory_blocks: &prepared.memory_blocks,
        tools: request.tools,
        images: request.images,
        activity_context: (!prepared.activity_context.is_empty())
            .then_some(prepared.activity_context.as_str()),
        max_output_tokens: prepared.max_output_tokens,
    }
}
impl RuntimeContext for ConversationContext<'_> {
    fn check_cancelled(&self) -> Result<(), String> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            Err("Request cancelled".into())
        } else {
            Ok(())
        }
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
        let cancelled = self.cancelled;
        let interrupted = &|| {
            interrupted() || cancelled.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        };
        let limits = agent.model_limits()?;
        let mut prepared = self.prepared();
        if let Some(limits) = limits {
            limits.validate()?;
            prepared.max_output_tokens = Some(
                self.policy
                    .output_reserve_tokens
                    .min(limits.max_output_tokens),
            );
        }
        let estimate = agent
            .estimate_request_with_control(&agent_request(&prepared, &request), interrupted)?;
        let Some(limits) = limits else {
            events(&ToolEvent::ContextStatus { status:ContextStatus{available:false,message:"Automatic compaction unavailable: configure this model's context and output limits".into(),estimated:estimate.as_ref().is_some_and(|e|e.estimated),input_tokens:estimate.as_ref().map(|e|e.tokens),max_tokens:None} });
            return Ok(prepared);
        };
        limits.validate()?;
        let Some(estimate) = estimate else {
            events(&ToolEvent::ContextStatus { status:ContextStatus{available:false,message:"Automatic compaction unavailable: token accounting unsupported for this input".into(),estimated:false,input_tokens:None,max_tokens:Some(limits.context_window)} });
            return Ok(prepared);
        };
        let reserve = self
            .policy
            .output_reserve_tokens
            .min(limits.max_output_tokens);
        let threshold = limits
            .context_window
            .saturating_mul(self.policy.auto_trigger_percent)
            / 100;
        let should_compact = estimate.tokens.saturating_add(reserve) >= threshold
            || request
                .latest_usage
                .or_else(|| {
                    self.conversation.as_ref().and_then(|c| {
                        let newer = c.memory.as_ref().is_none_or(|m| {
                            c.messages
                                .last()
                                .is_some_and(|last| last.created_at > m.created_at)
                        });
                        newer
                            .then(|| c.activities.last().and_then(|a| a.usage.as_ref()))
                            .flatten()
                    })
                })
                .is_some_and(|u| u.model == agent.model_name() && u.total_tokens >= threshold);
        if should_compact && self.conversation.is_some() {
            self.compact_with_control(
                agent,
                ContextPreparationRequest {
                    input: request.input,
                    tools: request.tools,
                    images: request.images,
                    latest_usage: request.latest_usage,
                },
                CompactionTrigger::Automatic,
                events,
                interrupted,
            )?;
            prepared = self.prepared();
        }
        prepared.max_output_tokens = Some(reserve);
        let next = agent
            .estimate_request_with_control(&agent_request(&prepared, &request), interrupted)?
            .ok_or("Token accounting became unavailable")?;
        if next.tokens.saturating_add(reserve) >= limits.context_window {
            return Err("Request exceeds model context; the latest messages and current input were preserved. Reduce the input or choose a larger model.".into());
        }
        events(&ToolEvent::ContextStatus{status:ContextStatus{available:true,message:format!("Automatic compaction at {}%; next-input estimate includes system, history, memory and tools",self.policy.auto_trigger_percent),estimated:next.estimated,input_tokens:Some(next.tokens),max_tokens:Some(limits.context_window)}});
        Ok(prepared)
    }
}
