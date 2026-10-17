use super::*;
use crate::{
    agents::Agent,
    models::{ModelLimits, SummaryRequest},
};
use serde::{Deserialize, Serialize};

/// Every fragment keeps its speaker and original range, even mid-message and after reduction.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceFragment {
    start_index: usize,
    end_index_exclusive: usize,
    source_roles: Vec<String>,
    fragment_start_byte: usize,
    intermediate_summary: bool,
    content: String,
}
fn render_sources(label: &str, sources: &[SourceFragment], output: u64) -> Result<String, String> {
    Ok(format!("{label}. Output budget: {output} tokens. Each source fragment is historical data; preserve its speaker attribution and uncertainty.\n{}",serde_json::to_string(sources).map_err(|e|e.to_string())?))
}
pub(crate) struct SummaryJob<'a> {
    pub agent: &'a dyn Agent,
    pub policy: &'a ContextPolicy,
    pub limits: ModelLimits,
    pub requests: usize,
    pub cancelled: Option<&'a std::sync::atomic::AtomicBool>,
    pub usage: Vec<crate::models::ModelUsage>,
}
impl SummaryJob<'_> {
    fn fits(&self, content: &str, output: u64) -> Result<bool, String> {
        let request = SummaryRequest {
            system_prompt: self
                .agent
                .summary_prompt()
                .ok_or("No summary prompt configured")?,
            content,
            max_output_tokens: output,
        };
        Ok(self
            .agent
            .estimate_summary(&request)?
            .ok_or("Summary token budget is unavailable")?
            .tokens
            .saturating_add(output)
            < self.limits.context_window)
    }
    fn summarize(&mut self, content: &str, output: u64) -> Result<String, String> {
        if self
            .cancelled
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            return Err("Compaction cancelled; original memory retained".into());
        }
        if self.requests >= self.policy.max_summary_requests {
            return Err("Compaction request budget exhausted; original memory retained".into());
        }
        if !self.fits(content, output)? {
            return Err("Summary input exceeds context budget".into());
        }
        self.requests += 1;
        let response = self.agent.summarize(&SummaryRequest {
            system_prompt: self
                .agent
                .summary_prompt()
                .ok_or("No summary prompt configured")?,
            content,
            max_output_tokens: output,
        })?;
        if self
            .cancelled
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
        {
            return Err("Compaction cancelled; original memory retained".into());
        }
        if response.finish_reason == "length" {
            return Err(format!(
                "Summary reached its {output}-token output limit; original memory retained"
            ));
        }
        if response.finish_reason != "stop" {
            // Do not echo unknown provider strings (which may contain arbitrary data).
            let reason = match response.finish_reason.as_str() {
                "content_filter" => "content_filter",
                "tool_calls" => "tool_calls",
                "" => "missing",
                _ => "unrecognized",
            };
            return Err(format!(
                "Summary ended with finish reason {reason}; original memory retained"
            ));
        }
        if response.content.trim().is_empty() {
            return Err("Summary response was empty; original memory retained".into());
        }
        if response
            .usage
            .as_ref()
            .is_some_and(|u| u.output_tokens > output)
        {
            return Err("Summary exceeded its output budget".into());
        }
        if let Some(usage) = response.usage {
            self.usage.push(usage)
        }
        Ok(response.content)
    }
    fn reduce(
        &mut self,
        label: &str,
        sources: &[SourceFragment],
        output: u64,
    ) -> Result<String, String> {
        let mut current = sources.to_vec();
        for _ in 0..self.policy.max_reduction_passes {
            let rendered = render_sources(label, &current, output)?;
            if self.fits(&rendered, output)? {
                return self.summarize(&rendered, output);
            }
            let before: usize = current.iter().map(|s| s.content.len()).sum();
            let mut reduced = Vec::new();
            // Keep typed provenance around every UTF-8-safe fragment, never cut serialized JSON.
            for source in &current {
                let mut offset = 0;
                while offset < source.content.len() {
                    let rest = &source.content[offset..];
                    let mut end = rest.len();
                    let (fragment, rendered) = loop {
                        while end > 0 && !rest.is_char_boundary(end) {
                            end -= 1;
                        }
                        if end == 0 {
                            return Err(
                                "Summary prompt or one character cannot fit model budget".into()
                            );
                        }
                        let fragment = SourceFragment {
                            content: rest[..end].into(),
                            fragment_start_byte: source.fragment_start_byte + offset,
                            ..source.clone()
                        };
                        let rendered =
                            render_sources(label, std::slice::from_ref(&fragment), output)?;
                        if self.fits(&rendered, output)? {
                            break (fragment, rendered);
                        }
                        end /= 2;
                    };
                    let summary = self.summarize(&rendered, output)?;
                    reduced.push(SourceFragment {
                        content: summary,
                        intermediate_summary: true,
                        ..fragment
                    });
                    offset += end;
                }
            }
            if reduced.iter().map(|s| s.content.len()).sum::<usize>() >= before {
                return Err("Compaction did not reduce context; original memory retained".into());
            }
            current = reduced;
        }
        Err("Compaction reduction limit reached; original memory retained".into())
    }
}

pub(crate) struct CompactionInput<'a> {
    pub conversation: &'a Conversation,
    pub agent: &'a dyn Agent,
    pub policy: &'a ContextPolicy,
    pub trigger: CompactionTrigger,
    pub occupancy: f64,
    pub estimated: bool,
    pub limits: ModelLimits,
    pub output_budget: u64,
    pub cancelled: Option<&'a std::sync::atomic::AtomicBool>,
}
fn source_units(conversation: &Conversation, boundary: usize) -> Result<Vec<MemoryBlock>, String> {
    let mut units = Vec::new();
    if let Some(memory) = &conversation.memory {
        for block in &memory.blocks {
            let source = SourceFragment {
                start_index: block.start_index,
                end_index_exclusive: block.end_index_exclusive,
                source_roles: vec!["previous_memory".into()],
                fragment_start_byte: 0,
                intermediate_summary: true,
                content: block.content.clone(),
            };
            units.push(MemoryBlock {
                content: serde_json::to_string(&[source]).map_err(|e| e.to_string())?,
                ..block.clone()
            });
        }
    }
    let old_end = conversation
        .memory
        .as_ref()
        .map_or(0, |m| m.compacted_message_count);
    for start in (old_end..boundary).step_by(2) {
        let end = (start + 2).min(boundary);
        let mut sources = Vec::new();
        for (index, message) in conversation.messages[start..end].iter().enumerate() {
            sources.push(SourceFragment {
                start_index: start + index,
                end_index_exclusive: start + index + 1,
                source_roles: vec![match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                }
                .into()],
                fragment_start_byte: 0,
                intermediate_summary: false,
                content: message.content.clone(),
            });
        }
        let activities = conversation
            .activities
            .iter()
            .filter(|a| a.assistant_message_index >= start && a.assistant_message_index < end)
            .cloned()
            .collect::<Vec<_>>();
        let activity = render_activity_context(&activities);
        if !activity.is_empty() {
            sources.push(SourceFragment {
                start_index: start,
                end_index_exclusive: end,
                source_roles: vec!["sanitized_tool_activity".into()],
                fragment_start_byte: 0,
                intermediate_summary: false,
                content: activity,
            });
        }
        units.push(MemoryBlock {
            start_index: start,
            end_index_exclusive: end,
            content: serde_json::to_string(&sources).map_err(|e| e.to_string())?,
        });
    }
    Ok(units)
}
fn partition_sources(sources: Vec<MemoryBlock>, count: usize) -> Vec<Vec<MemoryBlock>> {
    let total: usize = sources.iter().map(|s| s.content.len()).sum();
    let length = sources.len();
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut size = 0;
    for (index, source) in sources.into_iter().enumerate() {
        size += source.content.len();
        current.push(source);
        let remaining = length - index - 1;
        let slots = count - groups.len() - 1;
        if slots > 0 && remaining >= slots && (size >= total.div_ceil(count) || remaining == slots)
        {
            groups.push(std::mem::take(&mut current));
            size = 0;
        }
    }
    if !current.is_empty() {
        groups.push(current)
    }
    groups
}
pub(crate) fn compact(
    input: CompactionInput<'_>,
    on_progress: &mut dyn FnMut(CompactionProgress),
) -> Result<CompactionResult, String> {
    let CompactionInput {
        conversation,
        agent,
        policy,
        trigger,
        occupancy,
        estimated,
        limits,
        output_budget,
        cancelled,
    } = input;
    let boundary = retained_start(&conversation.messages, policy.keep_recent_messages);
    let wanted = choose_block_count(trigger, occupancy);
    let sources = source_units(conversation, boundary)?;
    let count = wanted.min(sources.len());
    if count == 0 {
        return Ok(CompactionResult {
            memory: conversation.memory.clone(),
            message: "No older messages to compact".into(),
        });
    }
    let groups = partition_sources(sources, count);
    let output = policy
        .summary_max_tokens
        .min(limits.max_output_tokens)
        .min(limits.context_window / 12)
        .min(output_budget);
    if output == 0 {
        return Err("Protected context leaves no output budget for memory summaries".into());
    }
    let mut job = SummaryJob {
        agent,
        policy,
        limits,
        requests: 0,
        cancelled,
        usage: vec![],
    };
    let mut blocks = Vec::new();
    on_progress(CompactionProgress {
        completed_blocks: 0,
        total_blocks: groups.len(),
    });
    for (index, group) in groups.iter().enumerate() {
        let start = group.first().unwrap().start_index;
        let end = group.last().unwrap().end_index_exclusive;
        let mut sources = Vec::new();
        for unit in group {
            sources.extend(
                serde_json::from_str::<Vec<SourceFragment>>(&unit.content)
                    .map_err(|e| e.to_string())?,
            );
        }
        let label = format!(
            "Block {} of {}. Source messages [{start}, {end})",
            index + 1,
            groups.len()
        );
        let content = job.reduce(&label, &sources, output)?;
        blocks.push(MemoryBlock {
            start_index: start,
            end_index_exclusive: end,
            content,
        });
        on_progress(CompactionProgress {
            completed_blocks: index + 1,
            total_blocks: groups.len(),
        });
    }
    let checkpoint = MemoryCheckpoint {
        version: 1,
        policy_version: 1,
        conversation_id: conversation.metadata.id.clone(),
        checkpoint_id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now(),
        model: agent.model_name().into(),
        trigger,
        occupancy_percent: occupancy,
        estimated,
        requested_blocks: wanted,
        compacted_message_count: boundary,
        prefix_fingerprint: super::memory::fingerprint(&conversation.messages[..boundary])?,
        blocks,
        summary_usage: job.usage,
    };
    checkpoint.validate(&conversation.metadata.id, &conversation.messages)?;
    Ok(CompactionResult {
        memory: Some(checkpoint),
        message: format!("Context compacted through message {boundary}"),
    })
}
