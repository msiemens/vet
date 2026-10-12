//! The pull-request queue: which pull requests belong on which list, and the facts each
//! row shows. Pure: the platforms only supply the pull requests.

use super::model::{
    ChangedLines, ChangesRequest, CiState, PullRequest, PullRequestQuery, QueueStatus, QueueTab, ReviewDecision,
    ReviewRequest, ReviewVerdict, Viewer,
};

/// What the viewer's earlier review says about the pull request as it is now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewState {
    NotReviewed,
    /// Reviewed, and nothing was pushed since.
    Reviewed(ReviewVerdict),
    /// Reviewed, and pushed to since (or the review was dismissed): "new pushes since
    /// your review".
    NewPushes(ReviewVerdict),
}

/// One line of the queue.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PullRequestRow {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub author_avatar: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub fork: Option<String>,
    /// The commit the pull request's head is at.
    pub head: String,
    pub lines: Option<ChangedLines>,
    pub comments: u32,
    pub ci: Option<CiState>,
    pub unresolved_threads: u32,
    pub review: ReviewState,
    pub waiting_for_me: bool,
    pub draft: bool,
    /// Where the pull request stands for the viewer; none when no status applies.
    pub status: Option<QueueStatus>,
    /// Who asked for changes, and how, when that is the status.
    pub changes_requested: Vec<ChangesRequest>,
    pub labels: Vec<String>,
    /// Unix seconds.
    pub updated_at: i64,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The review is asked of the viewer, directly or through a team of theirs.
fn requested_of(pr: &PullRequest, viewer: &Viewer) -> bool {
    pr.review_requests.iter().any(|request| match request {
        ReviewRequest::User(login) => same(login, &viewer.login),
        ReviewRequest::Team(team) => viewer.teams.iter().any(|t| same(t, team)),
    })
}

pub fn review_state(pr: &PullRequest) -> ReviewState {
    let Some(review) = &pr.viewer_review else {
        return ReviewState::NotReviewed;
    };
    let current = review.verdict != ReviewVerdict::Dismissed && review.commit.as_deref() == Some(pr.head.as_str());
    if current {
        ReviewState::Reviewed(review.verdict)
    } else {
        ReviewState::NewPushes(review.verdict)
    }
}

/// Review is requested from the viewer or their team, and the viewer has not reviewed
/// the pull request as it is now. A request outlives the review it was answered with
/// (a team's does), and a push does not renew it, so the review is compared with the head.
pub fn is_waiting_for_me(pr: &PullRequest, viewer: &Viewer) -> bool {
    requested_of(pr, viewer) && !matches!(review_state(pr), ReviewState::Reviewed(_))
}

/// Whether `pr` belongs to the list `query` asks for. The platforms narrow their
/// lists by the same criteria as far as they can; this is the rule.
pub fn matches(pr: &PullRequest, viewer: &Viewer, query: &PullRequestQuery) -> bool {
    let on_tab = match query.tab {
        QueueTab::WaitingForMe => is_waiting_for_me(pr, viewer),
        QueueTab::Mine => same(&pr.author, &viewer.login),
        QueueTab::Assigned => pr.assignees.iter().any(|a| same(a, &viewer.login)),
        QueueTab::AllOpen => true,
    };
    on_tab
        && query.author.as_deref().is_none_or(|a| same(a, &pr.author))
        && query
            .label
            .as_deref()
            .is_none_or(|l| pr.labels.iter().any(|x| same(x, l)))
        && query.draft.is_none_or(|d| d == pr.draft)
        && query.ci.is_none_or(|ci| pr.ci == Some(ci))
}

/// Where a pull request stands. A draft is a draft whatever else holds; then what holds the merge
/// back, then what the reviewers asked for, then what is asked of the viewer, then approval.
pub fn queue_status(draft: bool, blocked: bool, decision: ReviewDecision, waiting_for_me: bool) -> Option<QueueStatus> {
    if draft {
        Some(QueueStatus::Draft)
    } else if blocked {
        Some(QueueStatus::Blocked)
    } else if decision == ReviewDecision::ChangesRequested {
        Some(QueueStatus::ChangesRequested)
    } else if waiting_for_me {
        Some(QueueStatus::WaitingForYou)
    } else if decision == ReviewDecision::Approved {
        Some(QueueStatus::Approved)
    } else {
        None
    }
}

fn row(pr: PullRequest, viewer: &Viewer) -> PullRequestRow {
    let waiting_for_me = is_waiting_for_me(&pr, viewer);
    let status = queue_status(pr.draft, pr.blocked, pr.decision, waiting_for_me);
    PullRequestRow {
        review: review_state(&pr),
        waiting_for_me,
        status,
        changes_requested: if status == Some(QueueStatus::ChangesRequested) {
            pr.changes_requested
        } else {
            Vec::new()
        },
        number: pr.number,
        title: pr.title,
        url: pr.url,
        author: pr.author,
        author_avatar: pr.author_avatar,
        source_branch: pr.source_branch,
        target_branch: pr.target_branch,
        fork: pr.fork,
        head: pr.head,
        lines: pr.lines,
        comments: pr.comments,
        ci: pr.ci,
        unresolved_threads: pr.unresolved_threads,
        draft: pr.draft,
        labels: pr.labels,
        updated_at: pr.updated_at,
    }
}

/// The rows of the list `query` asks for, most recently updated first.
pub fn rows(prs: Vec<PullRequest>, viewer: &Viewer, query: &PullRequestQuery) -> Vec<PullRequestRow> {
    let mut rows: Vec<PullRequestRow> = prs
        .into_iter()
        .filter(|pr| matches(pr, viewer, query))
        .map(|pr| row(pr, viewer))
        .collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.number.cmp(&a.number)));
    rows.truncate(query.limit.map_or(usize::MAX, |n| n as usize));
    rows
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::platform::model::{ChangesAsked, ViewerReview};

    pub(crate) fn viewer() -> Viewer {
        Viewer {
            login: "Ada".to_owned(),
            teams: vec!["acme/reviewers".to_owned()],
        }
    }

    pub(crate) fn pr(number: u64) -> PullRequest {
        PullRequest {
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/acme/app/pull/{number}"),
            author: "grace".to_owned(),
            author_avatar: None,
            source_branch: "feature".to_owned(),
            target_branch: "main".to_owned(),
            fork: None,
            head: "c2".to_owned(),
            lines: None,
            comments: 0,
            draft: false,
            labels: Vec::new(),
            assignees: Vec::new(),
            ci: None,
            unresolved_threads: 0,
            review_requests: Vec::new(),
            viewer_review: None,
            decision: ReviewDecision::Undecided,
            changes_requested: Vec::new(),
            blocked: false,
            updated_at: 1_000 + number as i64,
        }
    }

    fn requested_user(mut pr: PullRequest) -> PullRequest {
        pr.review_requests = vec![ReviewRequest::User("ada".to_owned())];
        pr
    }

    fn reviewed(mut pr: PullRequest, verdict: ReviewVerdict, commit: &str) -> PullRequest {
        pr.viewer_review = Some(ViewerReview {
            verdict,
            commit: Some(commit.to_owned()),
        });
        pr
    }

    #[test]
    fn a_request_to_the_viewer_or_to_their_team_is_waiting() {
        assert!(!is_waiting_for_me(&pr(1), &viewer()));
        assert!(is_waiting_for_me(&requested_user(pr(1)), &viewer()));

        let mut by_team = pr(2);
        by_team.review_requests = vec![ReviewRequest::Team("Acme/Reviewers".to_owned())];
        assert!(is_waiting_for_me(&by_team, &viewer()));

        let mut other_team = pr(3);
        other_team.review_requests = vec![
            ReviewRequest::Team("acme/security".to_owned()),
            ReviewRequest::User("linus".to_owned()),
        ];
        assert!(!is_waiting_for_me(&other_team, &viewer()));
    }

    #[test]
    fn a_review_of_the_current_head_ends_the_wait_and_a_push_renews_it() {
        let current = reviewed(requested_user(pr(1)), ReviewVerdict::Approved, "c2");
        assert!(!is_waiting_for_me(&current, &viewer()));
        assert_eq!(review_state(&current), ReviewState::Reviewed(ReviewVerdict::Approved));

        let pushed_since = reviewed(requested_user(pr(1)), ReviewVerdict::ChangesRequested, "c1");
        assert!(is_waiting_for_me(&pushed_since, &viewer()));
        assert_eq!(
            review_state(&pushed_since),
            ReviewState::NewPushes(ReviewVerdict::ChangesRequested)
        );
    }

    #[test]
    fn a_dismissed_review_does_not_count_even_at_the_head() {
        let dismissed = reviewed(requested_user(pr(1)), ReviewVerdict::Dismissed, "c2");
        assert!(is_waiting_for_me(&dismissed, &viewer()));
        assert_eq!(
            review_state(&dismissed),
            ReviewState::NewPushes(ReviewVerdict::Dismissed)
        );
    }

    #[test]
    fn a_review_that_does_not_say_what_it_covered_does_not_end_the_wait() {
        let mut unknown = requested_user(pr(1));
        unknown.viewer_review = Some(ViewerReview {
            verdict: ReviewVerdict::Commented,
            commit: None,
        });
        assert!(is_waiting_for_me(&unknown, &viewer()));
    }

    #[test]
    fn a_review_without_a_request_is_not_waiting_but_shows_new_pushes() {
        let pushed = reviewed(pr(1), ReviewVerdict::Approved, "c1");
        assert!(!is_waiting_for_me(&pushed, &viewer()));
        assert_eq!(review_state(&pushed), ReviewState::NewPushes(ReviewVerdict::Approved));
    }

    #[test]
    fn each_tab_lists_its_own_pull_requests() {
        let waiting = requested_user(pr(1));
        let mut mine = pr(2);
        mine.author = "ADA".to_owned();
        let mut assigned = pr(3);
        assigned.assignees = vec!["ada".to_owned()];
        let other = pr(4);
        let all = vec![waiting, mine, assigned, other];
        let numbers = |tab| -> Vec<u64> {
            rows(all.clone(), &viewer(), &PullRequestQuery::new(tab))
                .iter()
                .map(|r| r.number)
                .collect()
        };
        assert_eq!(numbers(QueueTab::WaitingForMe), [1]);
        assert_eq!(numbers(QueueTab::Mine), [2]);
        assert_eq!(numbers(QueueTab::Assigned), [3]);
        assert_eq!(numbers(QueueTab::AllOpen), [4, 3, 2, 1]);
    }

    #[test]
    fn filters_narrow_every_tab() {
        let mut a = pr(1);
        a.labels = vec!["Bug".to_owned()];
        a.draft = true;
        a.ci = Some(CiState::Failure);
        let mut b = pr(2);
        b.author = "linus".to_owned();
        b.ci = Some(CiState::Success);
        let all = vec![a, b];
        let numbers = |edit: &dyn Fn(&mut PullRequestQuery)| -> Vec<u64> {
            let mut query = PullRequestQuery::new(QueueTab::AllOpen);
            edit(&mut query);
            rows(all.clone(), &viewer(), &query).iter().map(|r| r.number).collect()
        };
        assert_eq!(numbers(&|q| q.author = Some("LINUS".into())), [2]);
        assert_eq!(numbers(&|q| q.label = Some("bug".into())), [1]);
        assert_eq!(numbers(&|q| q.draft = Some(false)), [2]);
        assert_eq!(numbers(&|q| q.ci = Some(CiState::Failure)), [1]);
        assert_eq!(
            numbers(&|q| q.ci = Some(CiState::Pending)),
            Vec::<u64>::new(),
            "a pull request without checks has no CI state to match"
        );
        assert_eq!(
            numbers(&|q| {
                q.draft = Some(true);
                q.author = Some("linus".into());
            }),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn rows_come_most_recently_updated_first_up_to_the_limit() {
        let mut old = pr(1);
        old.updated_at = 10;
        let mut new = pr(2);
        new.updated_at = 30;
        let mut tie_a = pr(3);
        tie_a.updated_at = 20;
        let mut tie_b = pr(4);
        tie_b.updated_at = 20;
        let mut query = PullRequestQuery::new(QueueTab::AllOpen);
        let all = vec![old, new, tie_a, tie_b];
        let numbers = |query: &PullRequestQuery| -> Vec<u64> {
            rows(all.clone(), &viewer(), query).iter().map(|r| r.number).collect()
        };
        assert_eq!(numbers(&query), [2, 4, 3, 1]);
        query.limit = Some(2);
        assert_eq!(numbers(&query), [2, 4]);
    }

    #[test]
    fn a_row_has_the_first_status_that_applies_in_the_order_draft_blocked_changes_requested_waiting_approved() {
        let mut p = requested_user(pr(1));
        p.draft = true;
        p.blocked = true;
        p.decision = ReviewDecision::ChangesRequested;
        let status = |p: &PullRequest| {
            let query = PullRequestQuery::new(QueueTab::AllOpen);
            rows(vec![p.clone()], &viewer(), &query)[0].status
        };
        assert_eq!(status(&p), Some(QueueStatus::Draft));
        p.draft = false;
        assert_eq!(status(&p), Some(QueueStatus::Blocked));
        p.blocked = false;
        assert_eq!(status(&p), Some(QueueStatus::ChangesRequested));
        p.decision = ReviewDecision::Approved;
        assert_eq!(
            status(&p),
            Some(QueueStatus::WaitingForYou),
            "what is asked of the viewer comes first"
        );
        p.review_requests.clear();
        assert_eq!(status(&p), Some(QueueStatus::Approved));
        p.decision = ReviewDecision::Undecided;
        assert_eq!(status(&p), None);
    }

    #[test]
    fn a_row_names_who_asked_for_changes_only_when_that_is_its_status() {
        let mut p = pr(1);
        p.decision = ReviewDecision::ChangesRequested;
        p.changes_requested = vec![ChangesRequest {
            reviewer: "Linus".to_owned(),
            how: ChangesAsked::Rejected,
        }];
        let row =
            |p: &PullRequest| rows(vec![p.clone()], &viewer(), &PullRequestQuery::new(QueueTab::AllOpen)).remove(0);
        assert_eq!(row(&p).changes_requested, p.changes_requested);
        p.draft = true;
        assert_eq!(row(&p).status, Some(QueueStatus::Draft));
        assert_eq!(
            row(&p).changes_requested,
            [],
            "the chip says draft, not who asked for changes"
        );
    }

    #[test]
    fn a_row_carries_what_the_list_shows() {
        let mut p = requested_user(pr(7));
        p.ci = Some(CiState::Pending);
        p.unresolved_threads = 3;
        p.labels = vec!["wip".to_owned()];
        p.viewer_review = Some(ViewerReview {
            verdict: ReviewVerdict::Commented,
            commit: Some("c1".to_owned()),
        });
        let [row] = &rows(vec![p], &viewer(), &PullRequestQuery::new(QueueTab::WaitingForMe))[..] else {
            panic!("one row")
        };
        assert_eq!(
            (
                row.title.as_str(),
                row.author.as_str(),
                row.source_branch.as_str(),
                row.target_branch.as_str(),
                row.ci,
                row.unresolved_threads,
                row.review,
                row.waiting_for_me,
                row.updated_at
            ),
            (
                "PR 7",
                "grace",
                "feature",
                "main",
                Some(CiState::Pending),
                3,
                ReviewState::NewPushes(ReviewVerdict::Commented),
                true,
                1_007
            )
        );
    }
}
