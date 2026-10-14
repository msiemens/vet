//! How the platforms are doing: one standing per account, updated by every call made to a platform
//! through [`Health::watch`], so that what the viewer is told of a platform does not depend on which
//! request happened to meet its failure.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use super::host::PlatformKind;
use super::model::PlatformError;

/// How the last call to an account's platform went.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Standing {
    Fine,
    /// The error is kept whole: a rate limit says when it ends.
    Failing(PlatformError),
}

impl Standing {
    pub fn error(&self) -> Option<&PlatformError> {
        match self {
            Self::Fine => None,
            Self::Failing(e) => Some(e),
        }
    }
}

/// A call to a platform and what came of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    pub account: String,
    pub kind: PlatformKind,
    /// The standing before the call; none if the account was not called before.
    pub before: Option<Standing>,
    pub now: Standing,
}

impl Outcome {
    /// The call changed how the account stands.
    pub fn changed(&self) -> bool {
        self.before.as_ref() != Some(&self.now)
    }
}

type Heard = Box<dyn Fn(&Outcome) + Send + Sync>;

pub struct Health {
    standing: Mutex<HashMap<String, Standing>>,
    heard: Heard,
}

impl Health {
    /// `heard` is told of every call that says how its platform is doing.
    pub fn new(heard: impl Fn(&Outcome) + Send + Sync + 'static) -> Self {
        Self {
            standing: Mutex::default(),
            heard: Box::new(heard),
        }
    }

    /// How `account` stands, if it was called.
    pub fn standing(&self, account: &str) -> Option<Standing> {
        self.standing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(account)
            .cloned()
    }

    /// `call`, a call to the platform of `account`, with its outcome noted. A pull request or a
    /// right that is not there says nothing of how the platform is doing.
    pub async fn watch<T>(
        &self,
        account: &str,
        kind: PlatformKind,
        call: impl Future<Output = Result<T, PlatformError>>,
    ) -> Result<T, PlatformError> {
        let result = call.await;
        let now = match &result {
            Ok(_) => Some(Standing::Fine),
            Err(PlatformError::NotFound(_) | PlatformError::Forbidden(_)) => None,
            Err(e) => Some(Standing::Failing(e.clone())),
        };
        if let Some(now) = now {
            let before = self
                .standing
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(account.to_owned(), now.clone());
            (self.heard)(&Outcome {
                account: account.to_owned(),
                kind,
                before,
                now,
            });
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn limited(retry_at: i64) -> PlatformError {
        PlatformError::RateLimited {
            message: "slow down".to_owned(),
            retry_at: Some(retry_at),
        }
    }

    fn recorded() -> (Health, Arc<Mutex<Vec<Outcome>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = heard.clone();
        let health = Health::new(move |o| sink.lock().unwrap().push(o.clone()));
        (health, heard)
    }

    async fn call(health: &Health, account: &str, result: Result<(), PlatformError>) {
        let _ = health.watch(account, PlatformKind::GitHub, async { result }).await;
    }

    #[tokio::test]
    async fn a_rate_limit_keeps_when_it_ends_until_a_call_succeeds() {
        let (health, heard) = recorded();

        call(&health, "github.com", Err(limited(1_700))).await;
        assert_eq!(
            health.standing("github.com"),
            Some(Standing::Failing(limited(1_700))),
            "the limit's end is not lost"
        );
        call(&health, "github.com", Ok(())).await;

        assert_eq!(health.standing("github.com"), Some(Standing::Fine));
        let changes: Vec<bool> = heard.lock().unwrap().iter().map(Outcome::changed).collect();
        assert_eq!(changes, [true, true]);
    }

    #[tokio::test]
    async fn the_same_failure_twice_is_heard_twice_and_changes_nothing_the_second_time() {
        let (health, heard) = recorded();
        let rejected = || Err(PlatformError::Rejected("expired".to_owned()));

        call(&health, "github.com", rejected()).await;
        call(&health, "github.com", rejected()).await;

        let changes: Vec<bool> = heard.lock().unwrap().iter().map(Outcome::changed).collect();
        assert_eq!(changes, [true, false]);
    }

    #[tokio::test]
    async fn a_missing_pull_request_does_not_make_a_platform_unwell_and_accounts_stand_apart() {
        let (health, heard) = recorded();

        call(&health, "github.com", Ok(())).await;
        call(&health, "github.com", Err(PlatformError::NotFound("#9".to_owned()))).await;
        call(
            &health,
            "ghe.example",
            Err(PlatformError::Unreachable("down".to_owned())),
        )
        .await;

        assert_eq!(health.standing("github.com"), Some(Standing::Fine));
        assert!(health.standing("ghe.example").unwrap().error().is_some());
        assert_eq!(heard.lock().unwrap().len(), 2);
    }
}
