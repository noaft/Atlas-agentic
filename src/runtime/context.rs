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
impl RuntimeContext for StaticContext<'_> {
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
pub struct ConversationContext<'a> {
    pub store: &'a ConversationStore,
    pub conversation: Option<Conversation>,
    pub policy: ContextPolicy,
    pub cancelled: Option<&'a std::sync::atomic::AtomicBool>,
}
impl ConversationContext<'_> {
    fn prepared(&self) -> PreparedContext {
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
            .estimate_request(&agent_request(&prepared, &request))?
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
            .estimate_request(&agent_request(&protected, &request))?
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
        let fit = agent.estimate_request(&agent_request(&next, &request));
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
    fn prepare(
        &mut self,
        agent: &dyn Agent,
        request: ContextPreparationRequest<'_>,
        events: &mut dyn FnMut(&ToolEvent),
    ) -> Result<PreparedContext, String> {
        let Some(limits) = agent.model_limits()? else {
            events(&ToolEvent::ContextStatus { status:ContextStatus{available:false,message:"Automatic compaction unavailable: configure this model's context and output limits".into(),estimated:false,input_tokens:None,max_tokens:None} });
            return Ok(self.prepared());
        };
        limits.validate()?;
        let mut prepared = self.prepared();
        let Some(estimate) = agent.estimate_request(&agent_request(&prepared, &request))? else {
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
            self.compact(
                agent,
                ContextPreparationRequest {
                    input: request.input,
                    tools: request.tools,
                    images: request.images,
                    latest_usage: request.latest_usage,
                },
                CompactionTrigger::Automatic,
                events,
            )?;
            prepared = self.prepared();
        }
        prepared.max_output_tokens = Some(reserve);
        let next = agent
            .estimate_request(&agent_request(&prepared, &request))?
            .ok_or("Token accounting became unavailable")?;
        if next.tokens.saturating_add(reserve) >= limits.context_window {
            return Err("Request exceeds model context; the latest messages and current input were preserved. Reduce the input or choose a larger model.".into());
        }
        events(&ToolEvent::ContextStatus{status:ContextStatus{available:true,message:format!("Automatic compaction at {}%; next-input estimate includes system, history, memory and tools",self.policy.auto_trigger_percent),estimated:next.estimated,input_tokens:Some(next.tokens),max_tokens:Some(limits.context_window)}});
        Ok(prepared)
    }
}
