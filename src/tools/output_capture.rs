use std::io::Read;

pub(super) fn capture_bounded_observed(
    mut reader: impl Read,
    limit: usize,
    mut observer: impl FnMut(&[u8]),
) -> Result<(Vec<u8>, bool), String> {
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed to read exec_command output: {error}"))?;
        if read == 0 {
            break;
        }
        observer(&buffer[..read]);
        let remaining = limit.saturating_sub(retained.len());
        let keep = read.min(remaining);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }

    Ok((retained, truncated))
}

#[cfg(test)]
fn capture_bounded(reader: impl Read, limit: usize) -> Result<(Vec<u8>, bool), String> {
    capture_bounded_observed(reader, limit, |_| {})
}
#[cfg(test)]
mod tests {
    use super::capture_bounded;

    #[test]
    fn drains_input_while_retaining_only_the_limit() {
        let input = vec![b'x'; 32 * 1024];

        let (captured, truncated) = capture_bounded(input.as_slice(), 17).unwrap();

        assert_eq!(captured, vec![b'x'; 17]);
        assert!(truncated);
    }
}
