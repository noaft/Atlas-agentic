//! Interruptible HTTP inference, with no detached reader or parallel model call.
use super::sse::{read_sse, SseEvent};
use std::{io::Cursor, time::Duration};

const INTERRUPTED: &str = "Inference interrupted";
const MAX_FRAME: usize = 64 * 1024 * 1024;

pub(super) fn read_response(
    build: impl FnOnce(reqwest::Client) -> reqwest::RequestBuilder,
    provider: &str,
    timeout: Option<Duration>,
    interrupted: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(SseEvent) -> Result<bool, String>,
) -> Result<(), String> {
    if interrupted() {
        return Err(INTERRUPTED.into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not initialize inference transport")?;
    runtime.block_on(async {
        let mut client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        if let Some(timeout) = timeout { client = client.timeout(timeout); }
        let client = client.build().map_err(|_| "Could not initialize inference client")?;
        let response = tokio::select! {
            biased;
            _ = cancelled(interrupted) => return Err(INTERRUPTED.into()),
            result = build(client).send() => result.map_err(|_| format!("{provider} stream request failed; check connection and endpoint"))?,
        };
        if !response.status().is_success() {
            return Err(format!("{provider} stream request returned HTTP {}", response.status()));
        }
        let mut response = response;
        let mut frames = Frames::default();
        loop {
            let chunk = tokio::select! {
                biased;
                _ = cancelled(interrupted) => return Err(INTERRUPTED.into()),
                result = response.chunk() => result.map_err(|_| "Provider stream could not be read")?,
            };
            match chunk {
                Some(bytes) => if !frames.push(&bytes, on_event)? {return Ok(());},
                None => {frames.finish(on_event)?; return Ok(());}
            }
        }
    })
}
async fn cancelled(interrupted: &dyn Fn() -> bool) {
    while !interrupted() {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Buffer through blank lines before passing complete UTF-8 frames to the shared decoder.
#[derive(Default)]
struct Frames {
    bytes: Vec<u8>,
    line_start: usize,
}
impl Frames {
    fn push(
        &mut self,
        bytes: &[u8],
        on_event: &mut dyn FnMut(SseEvent) -> Result<bool, String>,
    ) -> Result<bool, String> {
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.bytes.len().saturating_add(part.len()) > MAX_FRAME {
                return Err("Provider stream event exceeds the size limit".into());
            }
            self.bytes.extend_from_slice(part);
            if part.last() == Some(&b'\n') {
                let line = &self.bytes[self.line_start..];
                if line == b"\n" || line == b"\r\n" {
                    if !self.finish(on_event)? {
                        return Ok(false);
                    }
                } else {
                    self.line_start = self.bytes.len();
                }
            }
        }
        Ok(true)
    }
    fn finish(
        &mut self,
        on_event: &mut dyn FnMut(SseEvent) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let mut more = true;
        read_sse(&mut Cursor::new(&self.bytes), &mut |event| {
            more = on_event(event)?;
            Ok(more)
        })?;
        self.bytes.clear();
        self.line_start = 0;
        Ok(more)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_chunks_preserve_utf8_crlf_multiple_frames_and_eof() {
        let wire = "event: message\r\ndata: é🌍\r\ndata: second\r\n\r\ndata: next\n\ndata: eof";
        let mut frames = Frames::default();
        let mut seen = Vec::new();
        let mut capture = |event: SseEvent| {
            seen.push((event.event, event.data));
            Ok(true)
        };
        for byte in wire.as_bytes() {
            assert!(frames.push(&[*byte], &mut capture).unwrap());
        }
        frames.finish(&mut capture).unwrap();
        assert_eq!(
            seen,
            [
                (Some("message".into()), "é🌍\nsecond".into()),
                (None, "next".into()),
                (None, "eof".into())
            ]
        );
    }
    #[test]
    fn terminal_event_does_not_deliver_following_frame() {
        let mut frames = Frames::default();
        let mut count = 0;
        assert!(!frames
            .push(b"data: done\n\ndata: hidden\n\n", &mut |_| {
                count += 1;
                Ok(false)
            })
            .unwrap());
        assert_eq!(count, 1);
    }
}
