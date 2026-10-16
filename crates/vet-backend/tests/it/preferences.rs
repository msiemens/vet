//! What the window remembers (SPEC §9) over the protocol: per scope, kept across launches, and
//! announced to every connection that asked for it.

use std::sync::Arc;

use crate::common::{Client, config, handler};
use vet_backend::{Orchestrator, open_store};
use vet_protocol::{BackendSettings, Event, Request, Response, ServerMessage};

async fn values(client: &Client, scope: Option<&str>) -> Vec<(String, String)> {
    let request = Request::Preferences {
        scope: scope.map(Into::into),
    };
    match client.call(request).await.unwrap() {
        Response::Preferences { values } => values.into_iter().map(|p| (p.key, p.value)).collect(),
        other => panic!("{other:?}"),
    }
}

async fn set(client: &Client, scope: Option<&str>, key: &str, value: Option<&str>) {
    let request = Request::SetPreference {
        scope: scope.map(Into::into),
        key: key.into(),
        value: value.map(Into::into),
    };
    assert_eq!(client.call(request).await.unwrap(), Response::Done);
}

fn heard(client: &mut Client) -> Vec<Event> {
    std::iter::from_fn(|| client.inbox.try_recv().ok())
        .filter_map(|m| match m {
            ServerMessage::Event { event } => Some(event),
            _ => None,
        })
        .collect()
}

fn pair(key: &str, value: &str) -> (String, String) {
    (key.into(), value.into())
}

#[tokio::test]
async fn preferences_are_kept_per_scope_across_launches_and_announced_to_the_other_connection() {
    let (handler, data) = handler().await;
    let mut window = Client::new(&handler);
    let mut other = Client::new(&handler);
    window.call(Request::Subscribe).await.unwrap();
    other.call(Request::Subscribe).await.unwrap();
    assert!(values(&window, None).await.is_empty());
    assert!(values(&other, Some("/repo")).await.is_empty());

    set(&window, None, "theme", Some("\"dark\"")).await;
    set(&window, Some("/repo"), "tabs", Some("[1]")).await;
    set(&window, None, "wrap", Some("true")).await;
    set(&window, None, "wrap", None).await;

    assert_eq!(values(&other, None).await, [pair("theme", "\"dark\"")]);
    assert_eq!(values(&other, Some("/repo")).await, [pair("tabs", "[1]")]);
    assert!(values(&other, Some("/elsewhere")).await.is_empty());
    let global = |key: &str, value: Option<&str>| Event::PreferenceChanged {
        scope: None,
        key: key.into(),
        value: value.map(Into::into),
    };
    let expected = [
        global("theme", Some("\"dark\"")),
        Event::PreferenceChanged {
            scope: Some("/repo".into()),
            key: "tabs".into(),
            value: Some("[1]".into()),
        },
        global("wrap", Some("true")),
        global("wrap", None),
    ];
    assert_eq!(heard(&mut other), expected, "the other connection");
    assert_eq!(heard(&mut window), expected, "the requester");

    drop((window, other, handler));
    let again: Arc<Orchestrator> = Arc::new(Orchestrator::new(open_store(data.path()).await.unwrap(), config()));
    let client = Client::new(&again);
    assert_eq!(values(&client, None).await, [pair("theme", "\"dark\"")]);
    assert_eq!(values(&client, Some("/repo")).await, [pair("tabs", "[1]")]);
}

#[tokio::test]
async fn a_change_of_the_backends_settings_is_announced_to_the_other_connection() {
    let (handler, _data) = handler().await;
    let window = Client::new(&handler);
    let mut other = Client::new(&handler);
    other.call(Request::Subscribe).await.unwrap();
    other.call(Request::Settings).await.unwrap();

    let settings = BackendSettings {
        semantic_cache_mb: 32,
        finished_review_days: 3,
    };
    window.call(Request::SetSettings { settings }).await.unwrap();

    assert_eq!(heard(&mut other), [Event::SettingsChanged]);
    let answer = other.call(Request::Settings).await;
    assert!(
        matches!(answer, Ok(Response::Settings { settings: s }) if s == settings),
        "{answer:?}"
    );
}

#[tokio::test]
async fn reading_the_settings_or_preferences_does_not_subscribe() {
    let (handler, _data) = handler().await;
    let window = Client::new(&handler);
    let mut reader = Client::new(&handler);
    // Reading them is no longer what hears their changes.
    reader.call(Request::Preferences { scope: None }).await.unwrap();
    reader.call(Request::Settings).await.unwrap();

    window
        .call(Request::SetPreference {
            scope: None,
            key: "theme".into(),
            value: Some("\"dark\"".into()),
        })
        .await
        .unwrap();
    window
        .call(Request::SetSettings {
            settings: BackendSettings {
                semantic_cache_mb: 32,
                finished_review_days: 3,
            },
        })
        .await
        .unwrap();
    assert!(heard(&mut reader).is_empty(), "a read must not subscribe");

    // `Subscribe` is what hears them.
    reader.call(Request::Subscribe).await.unwrap();
    window
        .call(Request::SetSettings {
            settings: BackendSettings {
                semantic_cache_mb: 64,
                finished_review_days: 3,
            },
        })
        .await
        .unwrap();
    assert_eq!(heard(&mut reader), [Event::SettingsChanged]);
}
