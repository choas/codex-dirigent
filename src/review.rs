//! Cue conversation and explicit review state machine.

use crate::cue::Cue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub speaker: Speaker,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Ready,
    Running { run_id: u64 },
    Reviewing { run_id: u64 },
    Accepted,
    Rejected,
    Committed { commit: String },
}

/// Proof that the user accepted a particular working-tree diff.
///
/// Its fields are private so commit code can validate it but callers cannot
/// manufacture approval without moving a session through review.
#[derive(Debug, Clone)]
pub struct ReviewApproval {
    fingerprint: blake3::Hash,
}

impl ReviewApproval {
    #[must_use]
    pub fn matches(&self, diff: &str) -> bool {
        self.fingerprint == fingerprint(diff)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReviewError {
    #[error("this action is not available in the current review state")]
    InvalidTransition,
    #[error("a stale execution result was ignored")]
    StaleRun,
    #[error("the working tree changed after the displayed diff was generated")]
    DiffChanged,
    #[error("follow-up instructions cannot be empty")]
    EmptyFollowUp,
}

#[derive(Debug, Clone)]
pub struct Session {
    cue: Cue,
    state: SessionState,
    messages: Vec<Message>,
    review_diff: String,
    approval: Option<ReviewApproval>,
    next_run_id: u64,
}

impl Session {
    #[must_use]
    pub fn new(cue: Cue) -> Self {
        let initial_message = Message {
            speaker: Speaker::User,
            text: cue.instruction().to_owned(),
        };
        Self {
            cue,
            state: SessionState::Ready,
            messages: vec![initial_message],
            review_diff: String::new(),
            approval: None,
            next_run_id: 1,
        }
    }

    #[must_use]
    pub const fn cue(&self) -> &Cue {
        &self.cue
    }

    #[must_use]
    pub const fn state(&self) -> &SessionState {
        &self.state
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub(crate) fn user_follow_ups(&self) -> impl Iterator<Item = &str> {
        self.messages.iter().skip(1).filter_map(|message| {
            (message.speaker == Speaker::User).then_some(message.text.as_str())
        })
    }

    #[must_use]
    pub fn review_diff(&self) -> &str {
        &self.review_diff
    }

    pub(crate) fn recover_ready(cue: Cue, follow_ups: Vec<String>) -> Self {
        Self::recover_with_state(cue, follow_ups, SessionState::Ready, String::new())
    }

    pub(crate) fn recover_reviewing(
        cue: Cue,
        follow_ups: Vec<String>,
        review_diff: String,
    ) -> Self {
        let next_run_id = recovered_next_run_id(follow_ups.len());
        Self::recover_with_state(
            cue,
            follow_ups,
            SessionState::Reviewing {
                run_id: next_run_id - 1,
            },
            review_diff,
        )
    }

    pub(crate) fn recover_committed_branch(
        cue: Cue,
        follow_ups: Vec<String>,
        review_diff: String,
    ) -> Self {
        Self::recover_with_state(cue, follow_ups, SessionState::Accepted, review_diff)
    }

    pub(crate) fn recover_done(cue: Cue, follow_ups: Vec<String>, commit: String) -> Self {
        Self::recover_with_state(
            cue,
            follow_ups,
            SessionState::Committed { commit },
            String::new(),
        )
    }

    /// Begin the initial Codex run.
    ///
    /// # Errors
    ///
    /// Returns an error unless the cue is ready to run.
    pub fn begin_run(&mut self) -> Result<u64, ReviewError> {
        if self.state != SessionState::Ready {
            return Err(ReviewError::InvalidTransition);
        }
        Ok(self.start_run())
    }

    /// Record a matching execution result and enter review.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale result or when no run is active.
    pub fn finish_run(
        &mut self,
        run_id: u64,
        summary: impl Into<String>,
        diff: impl Into<String>,
    ) -> Result<(), ReviewError> {
        if self.state != (SessionState::Running { run_id }) {
            return if matches!(self.state, SessionState::Running { .. }) {
                Err(ReviewError::StaleRun)
            } else {
                Err(ReviewError::InvalidTransition)
            };
        }
        self.messages.push(Message {
            speaker: Speaker::Codex,
            text: summary.into(),
        });
        self.review_diff = diff.into();
        self.approval = None;
        self.state = SessionState::Reviewing { run_id };
        Ok(())
    }

    /// Start a refinement run while retaining the conversation.
    ///
    /// # Errors
    ///
    /// Returns an error unless a result is under review or when the follow-up
    /// instruction is empty.
    pub fn follow_up(&mut self, instruction: impl Into<String>) -> Result<u64, ReviewError> {
        if !matches!(self.state, SessionState::Reviewing { .. }) {
            return Err(ReviewError::InvalidTransition);
        }
        let instruction = instruction.into().trim().to_owned();
        if instruction.is_empty() {
            return Err(ReviewError::EmptyFollowUp);
        }
        self.messages.push(Message {
            speaker: Speaker::User,
            text: instruction,
        });
        Ok(self.start_run())
    }

    /// Return from a failed or cancelled run to the last safe actionable state.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale run identifier or if no run is active.
    pub fn execution_failed(
        &mut self,
        run_id: u64,
        message: impl Into<String>,
    ) -> Result<(), ReviewError> {
        if self.state != (SessionState::Running { run_id }) {
            return if matches!(self.state, SessionState::Running { .. }) {
                Err(ReviewError::StaleRun)
            } else {
                Err(ReviewError::InvalidTransition)
            };
        }
        self.messages.push(Message {
            speaker: Speaker::Codex,
            text: message.into(),
        });
        self.state = if self.review_diff.is_empty() {
            SessionState::Ready
        } else {
            SessionState::Reviewing { run_id }
        };
        Ok(())
    }

    /// Accept only the exact diff displayed for review.
    ///
    /// # Errors
    ///
    /// Returns an error unless the session is reviewing, or if the current
    /// working-tree diff differs from the displayed result.
    pub fn accept(&mut self, current_diff: &str) -> Result<&ReviewApproval, ReviewError> {
        if !matches!(self.state, SessionState::Reviewing { .. }) {
            return Err(ReviewError::InvalidTransition);
        }
        if fingerprint(&self.review_diff) != fingerprint(current_diff) {
            return Err(ReviewError::DiffChanged);
        }
        self.approval = Some(ReviewApproval {
            fingerprint: fingerprint(current_diff),
        });
        self.state = SessionState::Accepted;
        self.approval.as_ref().ok_or(ReviewError::InvalidTransition)
    }

    /// Mark the reviewed work rejected.
    ///
    /// # Errors
    ///
    /// Returns an error unless changes are under review or accepted.
    pub fn reject(&mut self) -> Result<(), ReviewError> {
        if !matches!(
            self.state,
            SessionState::Reviewing { .. } | SessionState::Accepted
        ) {
            return Err(ReviewError::InvalidTransition);
        }
        self.approval = None;
        self.state = SessionState::Rejected;
        Ok(())
    }

    #[must_use]
    pub fn approval(&self) -> Option<&ReviewApproval> {
        if self.state == SessionState::Accepted {
            self.approval.as_ref()
        } else {
            None
        }
    }

    /// Record the commit produced from the accepted review.
    ///
    /// # Errors
    ///
    /// Returns an error unless the session currently has accepted work.
    pub fn mark_committed(&mut self, commit: impl Into<String>) -> Result<(), ReviewError> {
        if self.approval().is_none() {
            return Err(ReviewError::InvalidTransition);
        }
        self.state = SessionState::Committed {
            commit: commit.into(),
        };
        Ok(())
    }

    pub(crate) fn mark_recovered_branch_merged(
        &mut self,
        commit: impl Into<String>,
    ) -> Result<(), ReviewError> {
        if self.state != SessionState::Accepted || self.approval.is_some() {
            return Err(ReviewError::InvalidTransition);
        }
        self.state = SessionState::Committed {
            commit: commit.into(),
        };
        Ok(())
    }

    fn start_run(&mut self) -> u64 {
        let run_id = self.next_run_id;
        self.next_run_id += 1;
        self.approval = None;
        self.state = SessionState::Running { run_id };
        run_id
    }

    fn recover_with_state(
        cue: Cue,
        follow_ups: Vec<String>,
        state: SessionState,
        review_diff: String,
    ) -> Self {
        let mut messages = Vec::with_capacity(follow_ups.len() + 1);
        messages.push(Message {
            speaker: Speaker::User,
            text: cue.instruction().to_owned(),
        });
        messages.extend(follow_ups.into_iter().map(|text| Message {
            speaker: Speaker::User,
            text,
        }));
        let next_run_id = recovered_next_run_id(messages.len().saturating_sub(1));
        Self {
            cue,
            state,
            messages,
            review_diff,
            approval: None,
            next_run_id,
        }
    }
}

fn recovered_next_run_id(follow_up_count: usize) -> u64 {
    u64::try_from(follow_up_count)
        .unwrap_or(u64::MAX - 1)
        .saturating_add(2)
}

fn fingerprint(diff: &str) -> blake3::Hash {
    blake3::hash(diff.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cue::CueTarget;

    fn session() -> Session {
        Session::new(Cue::new("make it clear", CueTarget::Repository).unwrap())
    }

    #[test]
    fn runs_reviews_refines_and_accepts_exact_diff() {
        let mut session = session();
        let first = session.begin_run().unwrap();
        session.finish_run(first, "done", "diff one").unwrap();
        let second = session.follow_up("add a test").unwrap();
        assert!(second > first);
        session.finish_run(second, "refined", "diff two").unwrap();
        assert_eq!(session.messages().len(), 4);
        assert!(session.accept("diff two").unwrap().matches("diff two"));
        assert_eq!(session.state(), &SessionState::Accepted);
    }

    #[test]
    fn blocks_stale_results_and_changed_diffs() {
        let mut session = session();
        let run = session.begin_run().unwrap();
        assert_eq!(
            session.finish_run(run + 1, "stale", "diff"),
            Err(ReviewError::StaleRun)
        );
        session.finish_run(run, "done", "reviewed").unwrap();
        assert!(matches!(
            session.accept("changed"),
            Err(ReviewError::DiffChanged)
        ));
        assert!(session.approval().is_none());
    }

    #[test]
    fn commit_requires_acceptance_and_reject_revokes_it() {
        let mut session = session();
        assert_eq!(
            session.mark_committed("abc"),
            Err(ReviewError::InvalidTransition)
        );
        let run = session.begin_run().unwrap();
        session.finish_run(run, "done", "diff").unwrap();
        session.accept("diff").unwrap();
        session.reject().unwrap();
        assert!(session.approval().is_none());
        assert_eq!(session.state(), &SessionState::Rejected);
    }

    #[test]
    fn failed_refinement_returns_to_review() {
        let mut session = session();
        let first = session.begin_run().unwrap();
        session.finish_run(first, "done", "diff").unwrap();
        let second = session.follow_up("try again").unwrap();
        session.execution_failed(second, "cancelled").unwrap();
        assert_eq!(session.state(), &SessionState::Reviewing { run_id: second });
        assert_eq!(session.review_diff(), "diff");
    }
}
