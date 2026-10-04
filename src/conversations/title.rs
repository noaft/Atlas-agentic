pub fn sanitize_title(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let unquoted = collapsed
        .trim_matches(|character| character == '\'' || character == '"')
        .trim();

    if unquoted.is_empty() {
        return None;
    }

    Some(
        unquoted
            .chars()
            .take(60)
            .collect::<String>()
            .trim()
            .to_string(),
    )
}
