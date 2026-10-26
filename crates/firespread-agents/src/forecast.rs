//! Ensemble members handed one by one to the [`Executor`], with burn counts
//! aggregated as they finish.

use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use firespread_ensemble::{
    EnsembleError, HourlyCounts, HourlyInputs, HourlyMemberSummary, HourlyPerturbation,
    HourlyStats, MemberDraw, S_PER_HOUR,
};
use firespread_spread::Workspace;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::executor::{Executor, panic_message};
use crate::{AgentError, Result};

/// The seed, size, horizons and perturbations of one ensemble.
#[derive(Debug, Clone, PartialEq)]
pub struct EnsembleSpec {
    pub seed: u64,
    pub members: u32,
    /// Forecast horizons, hours.
    pub horizons_h: Vec<u32>,
    pub perturbation: HourlyPerturbation,
}

impl EnsembleSpec {
    /// The largest horizon, hours.
    ///
    /// # Errors
    /// [`AgentError::Ensemble`] wrapping [`EnsembleError::Empty`] when
    /// `members` is 0, [`EnsembleError::TooManyMembers`] above `u16::MAX`, and
    /// [`EnsembleError::NoHorizons`] for an empty horizon list.
    pub fn validated_horizon(&self) -> Result<u32> {
        let err = |source| AgentError::Ensemble {
            stage: "validating the ensemble",
            source,
        };
        if self.members == 0 {
            return Err(err(EnsembleError::Empty));
        }
        let max = u32::from(u16::MAX);
        if self.members > max {
            return Err(err(EnsembleError::TooManyMembers {
                members: self.members,
                max,
            }));
        }
        self.horizons_h
            .iter()
            .copied()
            .max()
            .ok_or(err(EnsembleError::NoHorizons))
    }
}

/// What one member did.
#[derive(Debug)]
pub enum MemberOutcome {
    /// The member ran to the largest horizon.
    Done(HourlyMemberSummary),
    /// The member returned an error ([`AgentError::MemberFailed`]) or
    /// panicked ([`AgentError::MemberPanicked`]).
    Failed(Arc<AgentError>),
    /// The member saw the stop flag before it started and did not run.
    Skipped,
}

/// One member's outcome and the run's progress when it arrived.
#[derive(Debug)]
pub struct MemberProgress {
    pub member: u32,
    /// Members whose outcome has arrived, this one included.
    pub finished: u32,
    pub members: u32,
    pub outcome: MemberOutcome,
}

/// The result of [`run_members`].
#[derive(Debug)]
pub struct EnsembleOutcome {
    /// Burn counts over the completed members: `counts.members` is their
    /// number and `counts.summaries` holds their summaries in member order.
    pub counts: HourlyCounts,
    /// Members that completed, ascending.
    pub completed: Vec<u32>,
    /// Members that failed, ascending, with their errors.
    pub failures: Vec<(u32, Arc<AgentError>)>,
    pub requested: u32,
    /// True when the token was cancelled before every member was admitted.
    pub cancelled: bool,
    /// Wall time from the first admission to the last outcome, s.
    pub wall_s: f64,
}

type Slot = (Workspace, Vec<f32>);

struct Shared<M> {
    inputs: Arc<HourlyInputs>,
    perturbation: HourlyPerturbation,
    seed: u64,
    dx: f64,
    horizon_max: u32,
    horizons_s: Vec<f32>,
    counts: Vec<AtomicU16>,
    buffers: Mutex<Vec<Slot>>,
    stop: AtomicBool,
    member_fn: M,
}

impl<M> Shared<M>
where
    M: Fn(
        &HourlyInputs,
        &MemberDraw,
        &mut Workspace,
        &mut Vec<f32>,
        u32,
    ) -> firespread_ensemble::Result<HourlyStats>,
{
    fn run(&self, member: u32) -> firespread_ensemble::Result<HourlyMemberSummary> {
        let draw = self.perturbation.draw(self.seed, member, self.dx)?;
        let slot = self
            .buffers
            .lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .pop();
        let (mut workspace, mut arrival) = slot.unwrap_or_else(|| (Workspace::new(), Vec::new()));
        let stats = (self.member_fn)(
            &self.inputs,
            &draw,
            &mut workspace,
            &mut arrival,
            self.horizon_max,
        )?;
        let n = self.inputs.inputs().len();
        let horizon_max_s = self.horizons_s.iter().copied().fold(0.0, f32::max);
        let mut burned_cells = vec![0_usize; self.horizons_s.len()];
        for (cell, &t) in arrival.iter().enumerate() {
            if t > horizon_max_s {
                continue;
            }
            for (k, &h) in self.horizons_s.iter().enumerate() {
                if t <= h {
                    self.counts[k * n + cell].fetch_add(1, Ordering::Relaxed);
                    burned_cells[k] += 1;
                }
            }
        }
        self.buffers
            .lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .push((workspace, arrival));
        Ok(HourlyMemberSummary {
            draw,
            stats,
            burned_cells,
        })
    }

    /// Runs `member` unless the stop flag is set, with a panic mapped to
    /// [`AgentError::MemberPanicked`].
    fn guarded(&self, member: u32) -> MemberOutcome {
        if self.stop.load(Ordering::Acquire) {
            return MemberOutcome::Skipped;
        }
        match catch_unwind(AssertUnwindSafe(|| self.run(member))) {
            Ok(Ok(summary)) => MemberOutcome::Done(summary),
            Ok(Err(source)) => {
                MemberOutcome::Failed(Arc::new(AgentError::MemberFailed { member, source }))
            }
            Err(payload) => MemberOutcome::Failed(Arc::new(AgentError::MemberPanicked {
                member,
                message: panic_message(payload.as_ref()),
            })),
        }
    }
}

/// Runs the members `0..spec.members` of an hourly ensemble on `exec`, one
/// pool job per member. Member `i` takes the draws
/// `spec.perturbation.draw(spec.seed, i, dx)`, runs `member_fn` to the
/// largest horizon, and adds 1 to `counts[k][cell]` for each horizon `k`
/// with arrival time at `cell <= 3600 horizons_h[k]` s, as
/// [`firespread_ensemble::run_hourly_ensemble`] does. A member is admitted
/// only after it holds an admission permit of `exec`. A member that returns
/// an error or panics is recorded in the outcome's failures and adds no
/// count; the other members run on. Calls `on_member` with each member's
/// outcome as it arrives.
///
/// Once `token` is cancelled no member is admitted, the stop flag is set so
/// admitted members that have not started return
/// [`MemberOutcome::Skipped`], and the members already running are awaited
/// before the call returns.
///
/// # Errors
/// The errors of [`EnsembleSpec::validated_horizon`],
/// [`AgentError::Ensemble`] wrapping [`EnsembleError::TooFewHours`] when the
/// weather holds fewer hours than the largest horizon or the errors of
/// [`HourlyPerturbation::draw`], and [`AgentError::AdmissionClosed`].
pub async fn run_members<M, P, Fut>(
    exec: &Executor,
    inputs: Arc<HourlyInputs>,
    spec: &EnsembleSpec,
    member_fn: M,
    token: &CancellationToken,
    mut on_member: P,
) -> Result<EnsembleOutcome>
where
    M: Fn(
            &HourlyInputs,
            &MemberDraw,
            &mut Workspace,
            &mut Vec<f32>,
            u32,
        ) -> firespread_ensemble::Result<HourlyStats>
        + Send
        + Sync
        + 'static,
    P: FnMut(MemberProgress) -> Fut,
    Fut: Future<Output = ()>,
{
    let horizon_max = spec.validated_horizon()?;
    let available = inputs.weather().hours.len();
    if usize::try_from(horizon_max).unwrap_or(usize::MAX) > available {
        return Err(AgentError::Ensemble {
            stage: "validating the ensemble",
            source: EnsembleError::TooFewHours {
                available,
                horizon_h: horizon_max,
            },
        });
    }
    let dx = f64::from(inputs.inputs().grid().dx_m());
    spec.perturbation
        .draw(spec.seed, 0, dx)
        .map_err(|source| AgentError::Ensemble {
            stage: "validating the perturbation",
            source,
        })?;
    let n = inputs.inputs().len();
    let shared = Arc::new(Shared {
        perturbation: spec.perturbation,
        seed: spec.seed,
        dx,
        horizon_max,
        horizons_s: spec
            .horizons_h
            .iter()
            .map(|&h| h as f32 * S_PER_HOUR)
            .collect(),
        counts: (0..n * spec.horizons_h.len())
            .map(|_| AtomicU16::new(0))
            .collect(),
        buffers: Mutex::new(Vec::new()),
        stop: AtomicBool::new(false),
        member_fn,
        inputs,
    });

    let members = spec.members;
    let mut summaries: Vec<Option<HourlyMemberSummary>> = (0..members).map(|_| None).collect();
    let mut failures = Vec::new();
    let mut in_flight: JoinSet<(u32, MemberOutcome)> = JoinSet::new();
    let mut next = 0_u32;
    let mut finished = 0_u32;
    let mut cancelled = false;
    let start = Instant::now();

    loop {
        if (next >= members || cancelled) && in_flight.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            () = token.cancelled(), if !cancelled => {
                cancelled = true;
                shared.stop.store(true, Ordering::Release);
                tracing::info!(admitted = next, members, "forecast cancelled");
            }
            Some(joined) = in_flight.join_next() => {
                let (member, outcome) = joined.unwrap_or_else(|e| {
                    (u32::MAX, MemberOutcome::Failed(Arc::new(AgentError::Panicked {
                        stage: "member task",
                        message: e.to_string(),
                    })))
                });
                finished += 1;
                match &outcome {
                    MemberOutcome::Done(summary) => {
                        if let Some(slot) = summaries.get_mut(member as usize) {
                            *slot = Some(summary.clone());
                        }
                    }
                    MemberOutcome::Failed(e) => {
                        tracing::warn!(member, error = %e, "member failed");
                        failures.push((member, Arc::clone(e)));
                    }
                    MemberOutcome::Skipped => {}
                }
                on_member(MemberProgress { member, finished, members, outcome }).await;
            }
            permit = exec.admit("member admission"), if next < members && !cancelled => {
                let permit = permit?;
                let member = next;
                next += 1;
                let job = Arc::clone(&shared);
                let fut = exec.submit(permit, move || job.guarded(member));
                in_flight.spawn(async move { (member, fut.await) });
            }
        }
    }

    let wall_s = start.elapsed().as_secs_f64();
    let shared = Arc::try_unwrap(shared).map_err(|_| AgentError::Panicked {
        stage: "collecting the counts",
        message: "a member job still holds the shared state".to_string(),
    })?;
    let flat: Vec<u16> = shared
        .counts
        .into_iter()
        .map(AtomicU16::into_inner)
        .collect();
    let mut completed = Vec::new();
    let mut done = Vec::new();
    for (member, summary) in summaries.into_iter().enumerate() {
        if let Some(s) = summary {
            completed.push(u32::try_from(member).unwrap_or(u32::MAX));
            done.push(s);
        }
    }
    failures.sort_by_key(|(m, _)| *m);
    Ok(EnsembleOutcome {
        counts: HourlyCounts {
            members: u32::try_from(done.len()).unwrap_or(u32::MAX),
            horizons_h: spec.horizons_h.clone(),
            counts: flat.chunks(n).map(<[u16]>::to_vec).collect(),
            summaries: done,
        },
        completed,
        failures,
        requested: members,
        cancelled,
        wall_s,
    })
}
