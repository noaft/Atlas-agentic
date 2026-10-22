use std::io::BufRead;
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}
/// Decode complete SSE frames. Protocol adapters must verify their own terminal event.
/// An EOF data frame is delivered too, for endpoints omitting the last blank line.
pub fn read_sse(
    reader: &mut dyn BufRead,
    on_event: &mut dyn FnMut(SseEvent) -> Result<bool, String>,
) -> Result<(), String> {
    let mut event = None;
    let mut data = Vec::new();
    loop {
        let mut line = String::new();
        let eof = reader
            .read_line(&mut line)
            .map_err(|_| "Provider stream could not be read".to_string())?
            == 0;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || eof {
            if !data.is_empty() {
                let frame = SseEvent {
                    event: event.take(),
                    data: data.join("\n"),
                };
                data.clear();
                if !on_event(frame)? {
                    return Ok(());
                }
            }
            event = None;
            if eof {
                return Ok(());
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        } else if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_string());
        }
        if data.iter().map(String::len).sum::<usize>() > 64 * 1024 * 1024 {
            return Err("Provider stream event exceeds the size limit".into());
        }
    }
}
