pub mod deepseek;

pub trait ModelProvider {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String, String>;
}
