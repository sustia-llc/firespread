//! One ensemble member: per-cell spread fields and one spread window.

use firespread_rothermel::DirectionalEnvironment;
use firespread_spread::{CellFields, FieldProvider, SpreadFields, WindowStats, Workspace};

use crate::inputs::{Inputs, NON_BURNABLE};
use crate::{EnsembleError, Result};

/// Wind of one member, uniform over the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberParams {
    /// Midflame wind speed, m/s.
    pub midflame_wind_m_s: f64,
    /// Azimuth the wind blows toward, rad clockwise from north.
    pub wind_toward_azimuth_rad: f64,
}

/// The fields of one member, evaluated per cell on demand: a
/// [`FieldProvider`] whose [`FieldProvider::fields`] is
/// [`MemberFields::eval`].
#[derive(Debug, Clone, Copy)]
pub struct MemberFields<'a> {
    inputs: &'a Inputs,
    params: MemberParams,
}

impl<'a> MemberFields<'a> {
    /// The fields of the member with wind `params` over `inputs`.
    #[must_use]
    pub fn new(inputs: &'a Inputs, params: MemberParams) -> Self {
        Self { inputs, params }
    }

    /// The head rate, eccentricity and head azimuth of cell `cell`: its fuel
    /// model's [`firespread_rothermel::DirectionalModel::eval`] at the
    /// member's wind and the cell's slope, each cast to `f32`, at a burnable
    /// cell, and all 0 at a non-burnable cell.
    ///
    /// # Errors
    /// The Rothermel error of the evaluation.
    ///
    /// # Panics
    /// When `cell >= inputs.len()`.
    pub fn eval(&self, cell: usize) -> Result<CellFields> {
        let inputs = self.inputs;
        let m = inputs.model_of_cell[cell];
        if m == NON_BURNABLE {
            return Ok(CellFields::default());
        }
        let out = inputs.models[usize::from(m)].eval(&DirectionalEnvironment {
            midflame_wind_m_s: self.params.midflame_wind_m_s,
            wind_toward_azimuth_rad: self.params.wind_toward_azimuth_rad,
            slope_tan: inputs.slope_tan[cell],
            upslope_azimuth_rad: inputs.upslope_azimuth_rad[cell],
        })?;
        Ok(CellFields {
            head_ros_m_s: out.head_ros_m_s as f32,
            eccentricity: out.eccentricity as f32,
            max_azimuth_rad: out.max_spread_azimuth_rad as f32,
        })
    }
}

impl FieldProvider for MemberFields<'_> {
    type Error = EnsembleError;

    fn fields(&mut self, cell: usize) -> Result<CellFields> {
        self.eval(cell)
    }
}

/// Per-member `f32` fields and arrival field, reused across members. The
/// field buffers are empty until [`MemberBuffers::fill_fields`].
#[derive(Debug, Clone, Default)]
pub struct MemberBuffers {
    head_ros_m_s: Vec<f32>,
    eccentricity: Vec<f32>,
    max_azimuth_rad: Vec<f32>,
    arrival_s: Vec<f32>,
}

impl MemberBuffers {
    /// Buffers with an arrival field sized for `inputs` and empty field
    /// buffers.
    #[must_use]
    pub fn new(inputs: &Inputs) -> Self {
        Self {
            head_ros_m_s: Vec::new(),
            eccentricity: Vec::new(),
            max_azimuth_rad: Vec::new(),
            arrival_s: vec![f32::INFINITY; inputs.len()],
        }
    }

    /// Sets each cell's head rate, eccentricity and head azimuth to
    /// [`MemberFields::eval`] of the cell. Resizes the buffers to `inputs`.
    ///
    /// # Errors
    /// The Rothermel error of a cell whose evaluation fails.
    pub fn fill_fields(&mut self, inputs: &Inputs, params: &MemberParams) -> Result<()> {
        let n = inputs.len();
        self.head_ros_m_s.resize(n, 0.0);
        self.eccentricity.resize(n, 0.0);
        self.max_azimuth_rad.resize(n, 0.0);
        let provider = MemberFields::new(inputs, *params);
        let cells = self
            .head_ros_m_s
            .iter_mut()
            .zip(self.eccentricity.iter_mut())
            .zip(self.max_azimuth_rad.iter_mut())
            .enumerate();
        for (i, ((head, ecc), az)) in cells {
            let f = provider.eval(i)?;
            (*head, *ecc, *az) = (f.head_ros_m_s, f.eccentricity, f.max_azimuth_rad);
        }
        Ok(())
    }

    /// Resets the arrival field to `inputs.initial_arrival_s()` and runs one
    /// spread window `[0, horizon_s]` over the fields of the last
    /// [`MemberBuffers::fill_fields`].
    ///
    /// # Errors
    /// The spread errors of [`Workspace::simulate_window`], including
    /// [`firespread_spread::SpreadError::FieldLength`] when the field buffers
    /// do not hold one value per cell of `inputs`.
    pub fn spread(
        &mut self,
        inputs: &Inputs,
        workspace: &mut Workspace,
        horizon_s: f32,
    ) -> Result<WindowStats> {
        self.reset_arrival(inputs);
        let fields = SpreadFields::new(
            inputs.grid(),
            &self.head_ros_m_s,
            &self.eccentricity,
            &self.max_azimuth_rad,
            inputs.burnable(),
        )?;
        Ok(workspace.simulate_window(&fields, &mut self.arrival_s, 0.0, horizon_s)?)
    }

    /// Resets the arrival field to `inputs.initial_arrival_s()` and runs one
    /// spread window `[0, horizon_s]` with each expanded cell's fields taken
    /// from [`MemberFields`] of `params` when the cell is expanded. Does not
    /// read or write the field buffers.
    ///
    /// # Errors
    /// The errors of [`Workspace::simulate_window_with`].
    pub fn spread_lazy(
        &mut self,
        inputs: &Inputs,
        params: &MemberParams,
        workspace: &mut Workspace,
        horizon_s: f32,
    ) -> Result<WindowStats> {
        self.reset_arrival(inputs);
        let mut provider = MemberFields::new(inputs, *params);
        workspace.simulate_window_with(
            inputs.grid(),
            inputs.burnable(),
            &mut provider,
            &mut self.arrival_s,
            0.0,
            horizon_s,
        )
    }

    fn reset_arrival(&mut self, inputs: &Inputs) {
        self.arrival_s.clear();
        self.arrival_s.extend_from_slice(inputs.initial_arrival_s());
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

    /// Arrival time per cell after the last spread window, s;
    /// `f32::INFINITY` is unburned.
    #[must_use]
    pub fn arrival_s(&self) -> &[f32] {
        &self.arrival_s
    }
}

/// Runs one member: [`MemberBuffers::spread_lazy`] to `horizon_s`. The
/// arrival field is [`MemberBuffers::arrival_s`] afterwards.
///
/// # Errors
/// The errors of [`MemberBuffers::spread_lazy`].
pub fn run_member(
    inputs: &Inputs,
    params: &MemberParams,
    workspace: &mut Workspace,
    buffers: &mut MemberBuffers,
    horizon_s: f32,
) -> Result<WindowStats> {
    buffers.spread_lazy(inputs, params, workspace, horizon_s)
}
