//! Seeded ensembles of members run in parallel.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};

use firespread_spread::Workspace;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, Normal};
use rayon::prelude::*;

use crate::inputs::Inputs;
use crate::member::{MemberBuffers, MemberParams, run_member};
use crate::{EnsembleError, Result};

/// Gaussian perturbation of a base wind: member speed
/// `max(base speed + N(0, speed_sd_m_s), 0)` and direction
/// `base azimuth + N(0, direction_sd_rad)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perturbation {
    pub base: MemberParams,
    /// Standard deviation of the speed perturbation, m/s.
    pub speed_sd_m_s: f64,
    /// Standard deviation of the direction perturbation, rad.
    pub direction_sd_rad: f64,
}

fn check(field: &'static str, value: f64, non_negative: bool) -> Result<f64> {
    if value.is_finite() && (!non_negative || value >= 0.0) {
        Ok(value)
    } else {
        Err(EnsembleError::InvalidPerturbation {
            field,
            bound: if non_negative {
                "a finite value >= 0"
            } else {
                "a finite value"
            },
            value,
        })
    }
}

impl Perturbation {
    /// The wind of member `member` of the ensemble seeded with `seed`: a
    /// `ChaCha8Rng` seeded by `seed_from_u64(seed)` on stream `member` draws
    /// the speed perturbation, then the direction perturbation.
    ///
    /// # Errors
    /// [`EnsembleError::InvalidPerturbation`] when the base speed or either
    /// standard deviation is negative or not finite, or the base azimuth is
    /// not finite.
    pub fn draw(&self, seed: u64, member: u32) -> Result<MemberParams> {
        let speed = check("base speed", self.base.midflame_wind_m_s, true)?;
        let azimuth = check("base azimuth", self.base.wind_toward_azimuth_rad, false)?;
        let speed_sd = check("speed sd", self.speed_sd_m_s, true)?;
        let direction_sd = check("direction sd", self.direction_sd_rad, true)?;
        let normal = |field, sd| {
            Normal::new(0.0, sd).map_err(|_| EnsembleError::InvalidPerturbation {
                field,
                bound: "a finite value >= 0",
                value: sd,
            })
        };
        let speed_noise = normal("speed sd", speed_sd)?;
        let direction_noise = normal("direction sd", direction_sd)?;
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        rng.set_stream(u64::from(member));
        let ds = speed_noise.sample(&mut rng);
        let dd = direction_noise.sample(&mut rng);
        Ok(MemberParams {
            midflame_wind_m_s: (speed + ds).max(0.0),
            wind_toward_azimuth_rad: azimuth + dd,
        })
    }
}

/// What one member drew and burned.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberSummary {
    pub params: MemberParams,
    /// Cells the spread search expanded.
    pub expanded: usize,
    /// Cells with arrival time `<= horizons_s[k]`, per horizon `k`.
    pub burned_cells: Vec<usize>,
}

/// Burn counts of an ensemble.
#[derive(Debug, Clone, PartialEq)]
pub struct Counts {
    pub members: u32,
    pub horizons_s: Vec<f32>,
    /// `counts[k][cell]`: members whose arrival time at `cell` is
    /// `<= horizons_s[k]`.
    pub counts: Vec<Vec<u16>>,
    /// One summary per member, by member index.
    pub summaries: Vec<MemberSummary>,
}

impl Counts {
    /// Burn probability `counts[k][cell] / members` per cell for horizon
    /// `k`, or `None` when `k` is out of range.
    #[must_use]
    pub fn probability(&self, k: usize) -> Option<Vec<f32>> {
        let members = self.members as f32;
        self.counts
            .get(k)
            .map(|c| c.iter().map(|&n| f32::from(n) / members).collect())
    }
}

type Slot = (Workspace, MemberBuffers);

/// Runs `members` members on the current rayon pool. Member `i` takes the
/// wind `perturbation.draw(seed, i)`, spreads from
/// `inputs.initial_arrival_s()` over one window `[0, max(horizons_s)]`, and
/// adds 1 to `counts[k][cell]` for each horizon `k` with arrival time at
/// `cell <= horizons_s[k]`. Counts are integer sums, so the result does not
/// depend on the pool's thread count or schedule.
///
/// # Errors
/// [`EnsembleError::Empty`] when `members` is 0,
/// [`EnsembleError::TooManyMembers`] above `u16::MAX`,
/// [`EnsembleError::NoHorizons`] and [`EnsembleError::InvalidHorizon`] for
/// an empty horizon list or a negative or non-finite horizon, the errors of
/// [`Perturbation::draw`], and [`EnsembleError::MemberFailed`] wrapping the
/// error of the lowest-indexed failing member.
pub fn run_ensemble(
    inputs: &Inputs,
    perturbation: &Perturbation,
    seed: u64,
    members: u32,
    horizons_s: &[f32],
) -> Result<Counts> {
    if members == 0 {
        return Err(EnsembleError::Empty);
    }
    let max = u32::from(u16::MAX);
    if members > max {
        return Err(EnsembleError::TooManyMembers { members, max });
    }
    if horizons_s.is_empty() {
        return Err(EnsembleError::NoHorizons);
    }
    if let Some((index, &value)) = horizons_s
        .iter()
        .enumerate()
        .find(|(_, h)| !(h.is_finite() && **h >= 0.0))
    {
        return Err(EnsembleError::InvalidHorizon { index, value });
    }
    perturbation.draw(seed, 0)?;
    let horizon_max = horizons_s.iter().copied().fold(0.0_f32, f32::max);
    let n = inputs.len();
    let counts: Vec<AtomicU16> = (0..n * horizons_s.len())
        .map(|_| AtomicU16::new(0))
        .collect();
    let pool: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

    let run = |member: u32| -> Result<MemberSummary> {
        let params = perturbation.draw(seed, member)?;
        let slot = pool
            .lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .pop();
        let (mut workspace, mut buffers) =
            slot.unwrap_or_else(|| (Workspace::new(), MemberBuffers::new(inputs)));
        let stats = run_member(inputs, &params, &mut workspace, &mut buffers, horizon_max)?;
        let mut burned_cells = vec![0_usize; horizons_s.len()];
        for (cell, &t) in buffers.arrival_s().iter().enumerate() {
            if t > horizon_max {
                continue;
            }
            for (k, &h) in horizons_s.iter().enumerate() {
                if t <= h {
                    counts[k * n + cell].fetch_add(1, Ordering::Relaxed);
                    burned_cells[k] += 1;
                }
            }
        }
        pool.lock()
            .map_err(|_| EnsembleError::BufferPoolPoisoned)?
            .push((workspace, buffers));
        Ok(MemberSummary {
            params,
            expanded: stats.expanded,
            burned_cells,
        })
    };

    let summaries = (0..members)
        .into_par_iter()
        .map(|member| {
            run(member).map_err(|e| EnsembleError::MemberFailed {
                member: member as usize,
                members: members as usize,
                reason: e.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let flat: Vec<u16> = counts.into_iter().map(AtomicU16::into_inner).collect();
    Ok(Counts {
        members,
        horizons_s: horizons_s.to_vec(),
        counts: flat.chunks(n).map(<[u16]>::to_vec).collect(),
        summaries,
    })
}
