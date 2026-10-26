//! One ensemble member: per-cell spread fields and one spread window.

use firespread_rothermel::DirectionalEnvironment;
use firespread_spread::{SpreadFields, WindowStats, Workspace};

use crate::Result;
use crate::inputs::{Inputs, NON_BURNABLE};

/// Wind of one member, uniform over the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberParams {
    /// Midflame wind speed, m/s.
    pub midflame_wind_m_s: f64,
    /// Azimuth the wind blows toward, rad clockwise from north.
    pub wind_toward_azimuth_rad: f64,
}

/// Per-member `f32` fields and arrival field, reused across members.
#[derive(Debug, Clone, Default)]
pub struct MemberBuffers {
    head_ros_m_s: Vec<f32>,
    eccentricity: Vec<f32>,
    max_azimuth_rad: Vec<f32>,
    arrival_s: Vec<f32>,
}

impl MemberBuffers {
    /// Buffers sized for `inputs`.
    #[must_use]
    pub fn new(inputs: &Inputs) -> Self {
        let n = inputs.len();
        Self {
            head_ros_m_s: vec![0.0; n],
            eccentricity: vec![0.0; n],
            max_azimuth_rad: vec![0.0; n],
            arrival_s: vec![f32::INFINITY; n],
        }
    }

    /// Sets each burnable cell's head rate, eccentricity and head azimuth to
    /// its fuel model's [`firespread_rothermel::DirectionalModel::eval`] at
    /// the member's wind and the cell's slope, each cast to `f32`, and each
    /// non-burnable cell's to 0. Resizes the buffers to `inputs`.
    ///
    /// # Errors
    /// The Rothermel error of a cell whose evaluation fails.
    pub fn fill_fields(&mut self, inputs: &Inputs, params: &MemberParams) -> Result<()> {
        let n = inputs.len();
        self.head_ros_m_s.resize(n, 0.0);
        self.eccentricity.resize(n, 0.0);
        self.max_azimuth_rad.resize(n, 0.0);
        let cells = self
            .head_ros_m_s
            .iter_mut()
            .zip(self.eccentricity.iter_mut())
            .zip(self.max_azimuth_rad.iter_mut())
            .enumerate();
        for (i, ((head, ecc), az)) in cells {
            let m = inputs.model_of_cell[i];
            if m == NON_BURNABLE {
                (*head, *ecc, *az) = (0.0, 0.0, 0.0);
                continue;
            }
            let out = inputs.models[usize::from(m)].eval(&DirectionalEnvironment {
                midflame_wind_m_s: params.midflame_wind_m_s,
                wind_toward_azimuth_rad: params.wind_toward_azimuth_rad,
                slope_tan: inputs.slope_tan[i],
                upslope_azimuth_rad: inputs.upslope_azimuth_rad[i],
            })?;
            *head = out.head_ros_m_s as f32;
            *ecc = out.eccentricity as f32;
            *az = out.max_spread_azimuth_rad as f32;
        }
        Ok(())
    }

    /// Resets the arrival field to `inputs.initial_arrival_s()` and runs one
    /// spread window `[0, horizon_s]` over the current fields.
    ///
    /// # Errors
    /// The spread errors of [`Workspace::simulate_window`].
    pub fn spread(
        &mut self,
        inputs: &Inputs,
        workspace: &mut Workspace,
        horizon_s: f32,
    ) -> Result<WindowStats> {
        self.arrival_s.clear();
        self.arrival_s.extend_from_slice(inputs.initial_arrival_s());
        let fields = SpreadFields::new(
            inputs.grid(),
            &self.head_ros_m_s,
            &self.eccentricity,
            &self.max_azimuth_rad,
            inputs.burnable(),
        )?;
        Ok(workspace.simulate_window(&fields, &mut self.arrival_s, 0.0, horizon_s)?)
    }

    /// Head rate of spread per cell, m/s.
    #[must_use]
    pub fn head_ros_m_s(&self) -> &[f32] {
        &self.head_ros_m_s
    }

    /// Ellipse eccentricity per cell.
    #[must_use]
    pub fn eccentricity(&self) -> &[f32] {
        &self.eccentricity
    }

    /// Head azimuth per cell, rad clockwise from north.
    #[must_use]
    pub fn max_azimuth_rad(&self) -> &[f32] {
        &self.max_azimuth_rad
    }

    /// Arrival time per cell after the last [`MemberBuffers::spread`], s;
    /// `f32::INFINITY` is unburned.
    #[must_use]
    pub fn arrival_s(&self) -> &[f32] {
        &self.arrival_s
    }
}

/// Runs one member: [`MemberBuffers::fill_fields`] then
/// [`MemberBuffers::spread`] to `horizon_s`. The arrival field is
/// [`MemberBuffers::arrival_s`] afterwards.
///
/// # Errors
/// The errors of the two steps.
pub fn run_member(
    inputs: &Inputs,
    params: &MemberParams,
    workspace: &mut Workspace,
    buffers: &mut MemberBuffers,
    horizon_s: f32,
) -> Result<WindowStats> {
    buffers.fill_fields(inputs, params)?;
    buffers.spread(inputs, workspace, horizon_s)
}
