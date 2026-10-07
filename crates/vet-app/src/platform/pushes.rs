//! The states of a pull request's source branch, for platforms that keep no list of
//! pushes (GitHub records only the force-pushes).

use super::model::{ForcePush, Push};

/// The source branch's states that `force_pushes` and the current head reveal, oldest
/// first. Between two force-pushes, ordinary pushes may have left states unknown to us;
/// the last one before a force-push is known, since the event names it.
pub fn pushes_from_force_pushes(force_pushes: &[ForcePush], current: &str) -> Vec<Push> {
    let mut ordered: Vec<&ForcePush> = force_pushes.iter().collect();
    ordered.sort_by_key(|f| f.at);
    let mut pushes: Vec<Push> = Vec::new();
    let mut add = |head: &str, at: Option<i64>, forced: bool| {
        if pushes.last().is_none_or(|last| last.head != head) {
            pushes.push(Push {
                head: head.to_owned(),
                at,
                forced,
            });
        }
    };
    for force_push in ordered {
        add(&force_push.before, None, false);
        add(&force_push.after, Some(force_push.at), true);
    }
    add(current, None, false);
    pushes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn force(at: i64, before: &str, after: &str) -> ForcePush {
        ForcePush {
            at,
            before: before.to_owned(),
            after: after.to_owned(),
        }
    }

    fn heads(pushes: &[Push]) -> Vec<(&str, bool)> {
        pushes.iter().map(|p| (p.head.as_str(), p.forced)).collect()
    }

    #[test]
    fn a_branch_never_force_pushed_has_its_head_as_the_only_state() {
        assert_eq!(heads(&pushes_from_force_pushes(&[], "h1")), [("h1", false)]);
    }

    #[test]
    fn a_force_push_splits_the_history_and_later_pushes_extend_it() {
        let pushes = pushes_from_force_pushes(&[force(50, "a", "b")], "c");
        assert_eq!(heads(&pushes), [("a", false), ("b", true), ("c", false)]);
        assert_eq!(pushes[0].at, None);
        assert_eq!(pushes[1].at, Some(50));
    }

    #[test]
    fn force_pushes_in_any_order_chain_by_time_and_a_shared_state_is_listed_once() {
        let pushes = pushes_from_force_pushes(&[force(90, "b", "c"), force(50, "a", "b")], "c");
        assert_eq!(heads(&pushes), [("a", false), ("b", true), ("c", true)]);
    }
}
