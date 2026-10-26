//! The Weather, Forecast and Publisher agents and the events they exchange.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use firespread_ensemble::{
    DeadFuelMoisture, HourFields, HourlyConfig, HourlyInputs, WindowWeather, run_hourly_member,
    write_probability_cogs,
};
use firespread_grid::{ForecastWindow, MANIFEST_VERSION, Manifest, WindLimitPolicy};
use firespread_rothermel::WindLimit;
use kanal::AsyncReceiver;
use tokio_util::sync::CancellationToken;

use crate::bus::{Bus, Delivery, KanalBus};
use crate::executor::Executor;
use crate::forecast::{EnsembleOutcome, EnsembleSpec, MemberProgress, run_members};
use crate::loaders::GridInputs;
use crate::{AgentError, Result};

/// Mailbox of the Forecast agent.
pub const FORECAST: &str = "forecast";
/// Mailbox of the Publisher agent.
pub const PUBLISHER: &str = "publisher";
/// Mailbox the caller reads through [`crate::Forecaster::inbox`].
pub const INBOX: &str = "inbox";
/// Name of the Weather agent's span; it has no mailbox.
pub const WEATHER: &str = "weather";

/// What the runtime's agents send each other and the caller.
#[derive(Debug)]
pub enum Event {
    /// Weather to Forecast: the window's hour labels and gridMET moisture.
    /// Precedes the window's hours.
    WeatherWindow {
        window: String,
        hours_utc: Vec<String>,
        fm100_pct: Vec<f32>,
        fm1000_pct: Vec<f32>,
    },
    /// Weather to Forecast: the fields of hour `index`.
    WeatherHour { index: usize, fields: HourFields },
    /// Forecast to the inbox: one member's outcome.
    Progress(MemberProgress),
    /// Forecast to Publisher: the aggregated burn counts.
    Forecast(Box<EnsembleOutcome>),
    /// Publisher to the inbox: the files of the run.
    Published(Box<Published>),
    /// Any agent to the inbox: the agent stopped its run on `error`.
    Failed {
        agent: &'static str,
        error: AgentError,
    },
}

/// The files a run wrote and the outcome they were written from.
#[derive(Debug)]
pub struct Published {
    pub run_dir: PathBuf,
    /// `burn_probability_h<N>` to the COG path.
    pub outputs: BTreeMap<String, PathBuf>,
    pub manifest: PathBuf,
    pub outcome: EnsembleOutcome,
}

/// Where the Weather agent gets the window's weather.
#[derive(Debug)]
pub(crate) enum WeatherSource {
    /// [`WindowWeather::load`] of `window` with `hours` hours.
    Index {
        weather_dir: PathBuf,
        window: String,
        hours: usize,
        header: firespread_grid::GridHeader,
        moisture: DeadFuelMoisture,
    },
    /// Weather already in memory.
    Memory(WindowWeather),
}

/// The settings the Publisher writes into the manifest.
#[derive(Debug, Clone)]
pub(crate) struct PublishSpec {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub window: ForecastWindow,
    pub inputs: BTreeMap<String, String>,
    pub hrrr_source: String,
    pub seed: u64,
    pub wind_limit: WindLimit,
    pub horizons_h: Vec<u32>,
}

async fn report_failure(bus: &KanalBus<Event>, agent: &'static str, error: AgentError) {
    tracing::error!(error = %error, "agent stopped its run");
    if let Err(e) = bus
        .send(Delivery {
            recipient: INBOX.to_string(),
            message: Event::Failed { agent, error },
        })
        .await
    {
        tracing::warn!(error = %e, "inbox closed");
    }
}

/// Receives until cancellation or a closed mailbox, discarding messages.
async fn idle(rx: &AsyncReceiver<Delivery<Event>>, token: &CancellationToken) {
    loop {
        tokio::select! {
            biased;
            () = token.cancelled() => return,
            d = rx.recv() => match d {
                Ok(d) => tracing::debug!(recipient = %d.recipient, "message after the run, discarded"),
                Err(_) => return,
            },
        }
    }
}

/// Loads the window's weather and sends it to Forecast: one
/// [`Event::WeatherWindow`] then one [`Event::WeatherHour`] per hour in hour
/// order. Then waits for cancellation.
pub(crate) async fn weather_agent(
    bus: Arc<KanalBus<Event>>,
    exec: Arc<Executor>,
    window: String,
    source: WeatherSource,
    token: CancellationToken,
) {
    let load = async {
        match source {
            WeatherSource::Index {
                weather_dir,
                window,
                hours,
                header,
                moisture,
            } => exec
                .run("weather load", move || {
                    WindowWeather::load(&weather_dir, &window, hours, &header, &moisture)
                })
                .await?
                .map_err(|source| AgentError::Ensemble {
                    stage: "loading the weather",
                    source,
                }),
            WeatherSource::Memory(w) => Ok(w),
        }
    };
    let loaded = tokio::select! {
        biased;
        () = token.cancelled() => return,
        w = load => w,
    };
    match loaded {
        Err(e) => report_failure(&bus, WEATHER, e).await,
        Ok(w) => {
            tracing::info!(hours = w.hours.len(), window = %window, "weather loaded");
            let mut events = vec![Event::WeatherWindow {
                window,
                hours_utc: w.hours_utc,
                fm100_pct: w.fm100_pct,
                fm1000_pct: w.fm1000_pct,
            }];
            events.extend(
                w.hours
                    .into_iter()
                    .enumerate()
                    .map(|(index, fields)| Event::WeatherHour { index, fields }),
            );
            for message in events {
                if token.is_cancelled() {
                    return;
                }
                if let Err(e) = bus
                    .send(Delivery {
                        recipient: FORECAST.to_string(),
                        message,
                    })
                    .await
                {
                    report_failure(&bus, WEATHER, e).await;
                    break;
                }
            }
        }
    }
    token.cancelled().await;
}

/// What the Forecast agent works from.
pub(crate) struct ForecastCtx {
    pub bus: Arc<KanalBus<Event>>,
    pub exec: Arc<Executor>,
    pub grid: Arc<GridInputs>,
    pub hourly: HourlyConfig,
    pub spec: EnsembleSpec,
}

/// Collects the window's weather from the mailbox; `None` on cancellation.
async fn collect_weather(
    rx: &AsyncReceiver<Delivery<Event>>,
    token: &CancellationToken,
) -> Result<Option<WindowWeather>> {
    let mut header: Option<(Vec<String>, Vec<f32>, Vec<f32>)> = None;
    let mut hours: Vec<Option<HourFields>> = Vec::new();
    let mut got = 0_usize;
    loop {
        if let Some((hours_utc, _, _)) = &header
            && got == hours_utc.len()
        {
            break;
        }
        let delivery = tokio::select! {
            biased;
            () = token.cancelled() => return Ok(None),
            d = rx.recv() => d,
        };
        let Ok(delivery) = delivery else {
            return Err(AgentError::WeatherIncomplete {
                expected: header.as_ref().map_or(0, |h| h.0.len()),
                got,
            });
        };
        match delivery.message {
            Event::WeatherWindow {
                hours_utc,
                fm100_pct,
                fm1000_pct,
                ..
            } => {
                hours = (0..hours_utc.len()).map(|_| None).collect();
                header = Some((hours_utc, fm100_pct, fm1000_pct));
            }
            Event::WeatherHour { index, fields } => {
                let slot = hours
                    .get_mut(index)
                    .ok_or_else(|| AgentError::WeatherProtocol {
                        reason: format!("hour {index} outside the window or before it"),
                    })?;
                if slot.replace(fields).is_some() {
                    return Err(AgentError::WeatherProtocol {
                        reason: format!("hour {index} arrived twice"),
                    });
                }
                got += 1;
            }
            other => tracing::debug!(event = ?std::mem::discriminant(&other), "ignored"),
        }
    }
    let (hours_utc, fm100_pct, fm1000_pct) = header.ok_or(AgentError::WeatherIncomplete {
        expected: 0,
        got: 0,
    })?;
    Ok(Some(WindowWeather {
        hours: hours.into_iter().flatten().collect(),
        hours_utc,
        fm100_pct,
        fm1000_pct,
    }))
}

async fn forecast_once(
    ctx: &ForecastCtx,
    rx: &AsyncReceiver<Delivery<Event>>,
    token: &CancellationToken,
) -> Result<Option<EnsembleOutcome>> {
    let Some(weather) = collect_weather(rx, token).await? else {
        return Ok(None);
    };
    let grid = Arc::clone(&ctx.grid);
    let hourly = ctx.hourly.clone();
    let build = ctx.exec.run("building the hourly inputs", move || {
        HourlyInputs::from_layers(&grid.layers(), weather, &hourly)
    });
    let inputs = tokio::select! {
        biased;
        () = token.cancelled() => return Ok(None),
        r = build => r?.map_err(|source| AgentError::Ensemble {
            stage: "building the hourly inputs",
            source,
        })?,
    };
    tracing::info!(members = ctx.spec.members, "ensemble starting");
    let bus = Arc::clone(&ctx.bus);
    let outcome = run_members(
        &ctx.exec,
        Arc::new(inputs),
        &ctx.spec,
        run_hourly_member,
        token,
        move |progress| {
            let bus = Arc::clone(&bus);
            async move {
                if let Err(e) = bus
                    .send(Delivery {
                        recipient: INBOX.to_string(),
                        message: Event::Progress(progress),
                    })
                    .await
                {
                    tracing::debug!(error = %e, "progress not delivered");
                }
            }
        },
    )
    .await?;
    tracing::info!(
        completed = outcome.completed.len(),
        failed = outcome.failures.len(),
        cancelled = outcome.cancelled,
        wall_s = outcome.wall_s,
        "ensemble finished"
    );
    Ok((!outcome.cancelled).then_some(outcome))
}

/// Collects the window's weather, builds the hourly inputs on the executor,
/// runs the ensemble with [`run_members`] publishing each member's progress
/// to the inbox, and sends the outcome to Publisher. Then receives and
/// discards until cancellation.
pub(crate) async fn forecast_agent(
    ctx: ForecastCtx,
    rx: AsyncReceiver<Delivery<Event>>,
    token: CancellationToken,
) {
    match forecast_once(&ctx, &rx, &token).await {
        Ok(Some(outcome)) => {
            if let Err(e) = ctx
                .bus
                .send(Delivery {
                    recipient: PUBLISHER.to_string(),
                    message: Event::Forecast(Box::new(outcome)),
                })
                .await
            {
                report_failure(&ctx.bus, "forecast", e).await;
            }
        }
        Ok(None) => return,
        Err(e) => report_failure(&ctx.bus, "forecast", e).await,
    }
    idle(&rx, &token).await;
}

fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn write_run(
    spec: &PublishSpec,
    header: &firespread_grid::GridHeader,
    outcome: EnsembleOutcome,
) -> Result<Published> {
    if outcome.counts.members == 0 {
        return Err(AgentError::NoCompletedMembers {
            requested: outcome.requested,
        });
    }
    std::fs::create_dir_all(&spec.run_dir).map_err(|source| AgentError::Io {
        path: spec.run_dir.clone(),
        source,
    })?;
    let outputs =
        write_probability_cogs(&outcome.counts, header, &spec.run_dir).map_err(|source| {
            AgentError::Ensemble {
                stage: "writing the burn-probability COGs",
                source,
            }
        })?;
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        run_id: spec.run_id.clone(),
        window: spec.window.clone(),
        grid: *header,
        inputs: spec.inputs.clone(),
        hrrr_source: spec.hrrr_source.clone(),
        seed: spec.seed,
        wind_limit: match spec.wind_limit {
            WindLimit::None => WindLimitPolicy::None,
            WindLimit::Rothermel => WindLimitPolicy::Rothermel,
            WindLimit::EffectiveWind => WindLimitPolicy::EffectiveWind,
        },
        members: outcome.counts.members,
        horizons_h: spec.horizons_h.clone(),
        outputs: outputs
            .iter()
            .map(|(k, p)| (k.clone(), relative(p, &spec.run_dir)))
            .collect(),
    };
    let manifest_path = spec.run_dir.join("manifest.json");
    manifest
        .write(&manifest_path)
        .map_err(|source| AgentError::Grid {
            stage: "writing the manifest",
            source,
        })?;
    Ok(Published {
        run_dir: spec.run_dir.clone(),
        outputs,
        manifest: manifest_path,
        outcome,
    })
}

/// On each [`Event::Forecast`], writes one burn-probability COG per horizon
/// and `manifest.json` into the run directory on the executor and sends
/// [`Event::Published`] to the inbox, or [`Event::Failed`] when the outcome
/// has no completed member or a write fails. Runs until cancellation.
pub(crate) async fn publisher_agent(
    bus: Arc<KanalBus<Event>>,
    exec: Arc<Executor>,
    grid: Arc<GridInputs>,
    spec: PublishSpec,
    rx: AsyncReceiver<Delivery<Event>>,
    token: CancellationToken,
) {
    loop {
        let delivery = tokio::select! {
            biased;
            () = token.cancelled() => return,
            d = rx.recv() => d,
        };
        let Ok(delivery) = delivery else { return };
        let Event::Forecast(outcome) = delivery.message else {
            continue;
        };
        let spec = spec.clone();
        let header = grid.header;
        let written = tokio::select! {
            biased;
            () = token.cancelled() => return,
            r = exec.run("publishing", move || write_run(&spec, &header, *outcome)) => r.and_then(|r| r),
        };
        let message = match written {
            Ok(p) => {
                tracing::info!(run_dir = %p.run_dir.display(), "run published");
                Event::Published(Box::new(p))
            }
            Err(error) => Event::Failed {
                agent: PUBLISHER,
                error,
            },
        };
        if let Err(e) = bus
            .send(Delivery {
                recipient: INBOX.to_string(),
                message,
            })
            .await
        {
            tracing::warn!(error = %e, "inbox closed");
        }
    }
}
