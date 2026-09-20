use super::*;

fn event(line: &str) -> Option<PlayerEvent> {
    match parse_message(line)? {
        Message::Event(e) => Some(e),
        Message::Reply { .. } => None,
    }
}

#[test]
fn end_file_reason_is_preserved() {
    for (reason, expected) in [
        ("eof", EndReason::Eof),
        ("stop", EndReason::Stop),
        ("quit", EndReason::Quit),
        ("error", EndReason::Error),
        ("redirect", EndReason::Other),
    ] {
        let line = format!(r#"{{"event":"end-file","reason":"{reason}","playlist_entry_id":1}}"#);
        assert_eq!(
            event(&line),
            Some(PlayerEvent::EndFile(expected)),
            "{reason}"
        );
    }
}

#[test]
fn property_changes_become_events() {
    let line = |name: &str, data: &str| {
        format!(r#"{{"event":"property-change","id":1,"name":"{name}","data":{data}}}"#)
    };
    assert_eq!(
        event(&line("time-pos", "12.5")),
        Some(PlayerEvent::Position(12.5))
    );
    assert_eq!(
        event(&line("duration", "180")),
        Some(PlayerEvent::Duration(180.0))
    );
    assert_eq!(
        event(&line("pause", "true")),
        Some(PlayerEvent::Paused(true))
    );
    // Property became unavailable, or one we don't track.
    assert_eq!(event(&line("time-pos", "null")), None);
    assert_eq!(event(&line("volume", "50")), None);
    assert_eq!(
        event(r#"{"event":"property-change","id":1,"name":"pause"}"#),
        None
    );
}

#[test]
fn replies_and_noise() {
    assert_eq!(
        parse_message(r#"{"error":"success","request_id":4}"#),
        Some(Message::Reply {
            error: "success".into()
        })
    );
    assert_eq!(
        parse_message(r#"{"error":"property unavailable","request_id":5}"#),
        Some(Message::Reply {
            error: "property unavailable".into()
        })
    );
    assert_eq!(parse_message(r#"{"event":"idle"}"#), None);
    assert_eq!(parse_message("not json"), None);
}

#[test]
fn loadfile_without_headers_is_the_plain_form() {
    let target = StreamTarget {
        url: "/music/a.mp3".into(),
        headers: vec![],
    };
    assert_eq!(
        loadfile_command(&target),
        json!(["loadfile", "/music/a.mp3", "replace"])
    );
}

#[test]
fn loadfile_passes_headers_as_an_option() {
    let target = StreamTarget {
        url: "https://x/stream".into(),
        headers: vec![("Authorization".into(), "OAuth tok".into())],
    };
    assert_eq!(
        loadfile_command(&target),
        json!(["loadfile", "https://x/stream", "replace", -1,
               {"http-header-fields": "Authorization: OAuth tok"}])
    );
}
