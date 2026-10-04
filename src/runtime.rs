use serde::Serialize;

use crate::{agent_output::AgentOutput, agents::AgentRegistry, tools::ToolRegistry};

const MAX_TOOL_CALLS: usize = 8;

#[derive(Serialize)]
struct ToolExecutionRecord {
    tool: String,
    args: serde_json::Value,
    output: String,
}

#[derive(Serialize)]
struct ToolContinuation<'a> {
    #[serde(rename = "type")]
    message_type: &'static str,
    original_user_request: &'a str,
    tool_results: &'a [ToolExecutionRecord],
    instruction: &'static str,
}

pub struct Runtime {
    agents: AgentRegistry,
    tools: ToolRegistry,
}

impl Runtime {
    pub fn new(agents: AgentRegistry) -> Self {
        Self {
            agents,
            tools: ToolRegistry::empty(),
        }
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn ask_stream(
        &self,
        prompt: &str,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, String> {
        let agent = self
            .agents
            .default_agent()
            .ok_or_else(|| "no agent is registered".to_string())?;
        let tool_definitions = self.tools.definitions();

        if tool_definitions.is_empty() {
            let response = agent.process_stream(prompt, &tool_definitions, on_delta)?;
            let output = AgentOutput::parse(&response)?;
            return self.tools.execute(&output).map(|result| result.message);
        }

        let mut input = prompt.to_string();
        let mut tool_results = Vec::new();

        loop {
            let response = agent.process_stream(&input, &tool_definitions, &mut |_| {})?;
            let output = AgentOutput::parse_tool_protocol(&response)?;

            match output {
                AgentOutput::Final { content } => {
                    on_delta(&content);
                    return Ok(content);
                }
                AgentOutput::ToolCall { tool, args } => {
                    if tool_results.len() >= MAX_TOOL_CALLS {
                        return Err(format!(
                            "agent exceeded the maximum of {MAX_TOOL_CALLS} tool calls"
                        ));
                    }

                    let tool_call = AgentOutput::ToolCall {
                        tool: tool.clone(),
                        args: args.clone(),
                    };
                    let result = self.tools.execute(&tool_call)?;
                    tool_results.push(ToolExecutionRecord {
                        tool,
                        args,
                        output: result.message,
                    });
                    input = render_tool_continuation(prompt, &tool_results)?;
                }
            }
        }
    }
}

fn render_tool_continuation(
    original_user_request: &str,
    tool_results: &[ToolExecutionRecord],
) -> Result<String, String> {
    serde_json::to_string_pretty(&ToolContinuation {
        message_type: "tool_results",
        original_user_request,
        tool_results,
        instruction: "Use these tool results to continue the task. Return a final answer or request another available tool.",
    })
    .map_err(|error| format!("failed to serialize tool results: {error}"))
}
