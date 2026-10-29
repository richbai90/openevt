use crate::{
    algorithms::{
        EvtTransformer,
        tensor::{ChannelMode, Tensor, tensor_from_polarities},
    },
    types::{EventCD, EventCoordinate, EventTimestamp},
};

pub struct BaseTimeSurface {
    width: EventCoordinate,
    height: EventCoordinate,
    timestamps: Vec<EventTimestamp>,
}

impl BaseTimeSurface {
    pub fn new(width: EventCoordinate, height: EventCoordinate) -> Self {
        BaseTimeSurface {
            width,
            height,
            timestamps: vec![0; (width as usize) * (height as usize) * 2],
        }
    }

    pub fn update(&mut self, x: EventCoordinate, y: EventCoordinate, p: bool, ts: EventTimestamp) {
        let index = self.get_index(x, y, p);
        self.timestamps[index] = ts;
    }

    pub fn get(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> EventTimestamp {
        self.timestamps[self.get_index(x, y, p)]
    }

    pub fn width(&self) -> EventCoordinate {
        self.width
    }
    pub fn height(&self) -> EventCoordinate {
        self.height
    }

    /// Materializes the stored per-pixel last-event timestamps as a tensor.
    ///
    /// In single-channel mode the combined pixel is the most recent (maximum)
    /// timestamp across both polarities.
    pub fn to_tensor(&self, mode: ChannelMode) -> Tensor {
        tensor_from_polarities(
            self.width,
            self.height,
            mode,
            |x, y| self.get(x, y, true) as f64,
            |x, y| self.get(x, y, false) as f64,
            |pos, neg| pos.max(neg),
        )
    }

    #[inline]
    fn get_index(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> usize {
        ((y as usize * self.width as usize) + x as usize) * 2 + (p as usize)
    }
}

pub struct LinearDecayTimeSurface {
    surface: BaseTimeSurface,
    decay_time: EventTimestamp,
    last_update_t: EventTimestamp,
    values: Vec<f64>,
}

impl LinearDecayTimeSurface {
    pub fn new(
        width: EventCoordinate,
        height: EventCoordinate,
        decay_time: EventTimestamp,
    ) -> Self {
        Self {
            surface: BaseTimeSurface::new(width, height),
            decay_time,
            last_update_t: 0,
            values: vec![0.0; (width as usize) * (height as usize) * 2],
        }
    }

    pub fn update(&mut self, x: EventCoordinate, y: EventCoordinate, p: bool, ts: EventTimestamp) {
        self.decay_to(ts);

        let index = self.surface.get_index(x, y, p);
        self.surface.update(x, y, p, ts);
        self.values[index] = 1.0;
    }

    pub fn decay_to(&mut self, ts: EventTimestamp) {
        let elapsed = ts.saturating_sub(self.last_update_t);
        if elapsed == 0 {
            return;
        }

        if self.decay_time == 0 {
            self.values.fill(0.0);
        } else {
            let decrement = elapsed as f64 / self.decay_time as f64;
            for value in &mut self.values {
                *value = (*value - decrement).max(0.0);
            }
        }

        self.last_update_t = self.last_update_t.max(ts);
    }

    pub fn get(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> f64 {
        self.values[self.surface.get_index(x, y, p)]
    }

    pub fn timestamp(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> EventTimestamp {
        self.surface.get(x, y, p)
    }

    pub fn width(&self) -> EventCoordinate {
        self.surface.width()
    }

    pub fn height(&self) -> EventCoordinate {
        self.surface.height()
    }

    /// Materializes the linearly decayed per-pixel values as a tensor.
    ///
    /// Single-channel mode combines both polarities by summing the two
    /// per-pixel intensities.
    pub fn to_tensor(&self, mode: ChannelMode) -> Tensor {
        tensor_from_polarities(
            self.surface.width(),
            self.surface.height(),
            mode,
            |x, y| self.get(x, y, true),
            |x, y| self.get(x, y, false),
            |pos, neg| pos + neg,
        )
    }
}

pub struct BaseTimeSurfaceTransformer {
    width: EventCoordinate,
    height: EventCoordinate,
}

impl BaseTimeSurfaceTransformer {
    pub fn new(width: EventCoordinate, height: EventCoordinate) -> Self {
        Self { width, height }
    }
}

impl EvtTransformer for BaseTimeSurfaceTransformer {
    type Output = BaseTimeSurface;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output {
        let mut surface = BaseTimeSurface::new(self.width, self.height);
        for evt in events {
            surface.update(evt.x, evt.y, evt.p.into(), evt.t);
        }
        surface
    }
}

pub struct LinearDecayTimeSurfaceTransformer {
    width: EventCoordinate,
    height: EventCoordinate,
    decay_time: EventTimestamp,
}

impl LinearDecayTimeSurfaceTransformer {
    pub fn new(
        width: EventCoordinate,
        height: EventCoordinate,
        decay_time: EventTimestamp,
    ) -> Self {
        Self {
            width,
            height,
            decay_time,
        }
    }
}

impl EvtTransformer for LinearDecayTimeSurfaceTransformer {
    type Output = LinearDecayTimeSurface;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output {
        let mut surface = LinearDecayTimeSurface::new(self.width, self.height, self.decay_time);
        for evt in events {
            surface.update(evt.x, evt.y, evt.p.into(), evt.t);
        }
        surface
    }
}

pub struct ExponentialDecayTimeSurface {
    surface: BaseTimeSurface,
    tau: EventTimestamp,
    last_update_t: EventTimestamp,
    values: Vec<f64>,
}

impl ExponentialDecayTimeSurface {
    pub fn new(width: EventCoordinate, height: EventCoordinate, tau: EventTimestamp) -> Self {
        Self {
            surface: BaseTimeSurface::new(width, height),
            tau,
            last_update_t: 0,
            values: vec![0.0; (width as usize) * (height as usize) * 2],
        }
    }

    pub fn update(&mut self, x: EventCoordinate, y: EventCoordinate, p: bool, ts: EventTimestamp) {
        self.decay_to(ts);

        let index = self.surface.get_index(x, y, p);
        self.surface.update(x, y, p, ts);
        self.values[index] = 1.0;
    }

    pub fn decay_to(&mut self, ts: EventTimestamp) {
        let elapsed = ts.saturating_sub(self.last_update_t);
        if elapsed == 0 {
            return;
        }

        if self.tau == 0 {
            self.values.fill(0.0);
        } else {
            let multiplier = (-(elapsed as f64) / self.tau as f64).exp();
            for value in &mut self.values {
                *value *= multiplier;
            }
        }

        self.last_update_t = self.last_update_t.max(ts);
    }

    pub fn get(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> f64 {
        self.values[self.surface.get_index(x, y, p)]
    }

    pub fn timestamp(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> EventTimestamp {
        self.surface.get(x, y, p)
    }

    pub fn width(&self) -> EventCoordinate {
        self.surface.width()
    }

    pub fn height(&self) -> EventCoordinate {
        self.surface.height()
    }

    /// Materializes the exponentially decayed per-pixel values as a tensor.
    ///
    /// Single-channel mode combines both polarities by summing the two
    /// per-pixel intensities.
    pub fn to_tensor(&self, mode: ChannelMode) -> Tensor {
        tensor_from_polarities(
            self.surface.width(),
            self.surface.height(),
            mode,
            |x, y| self.get(x, y, true),
            |x, y| self.get(x, y, false),
            |pos, neg| pos + neg,
        )
    }
}

pub struct ExponentialDecayTimeSurfaceTransformer {
    width: EventCoordinate,
    height: EventCoordinate,
    tau: EventTimestamp,
}

impl ExponentialDecayTimeSurfaceTransformer {
    pub fn new(width: EventCoordinate, height: EventCoordinate, tau: EventTimestamp) -> Self {
        Self { width, height, tau }
    }
}

impl EvtTransformer for ExponentialDecayTimeSurfaceTransformer {
    type Output = ExponentialDecayTimeSurface;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output {
        let mut surface = ExponentialDecayTimeSurface::new(self.width, self.height, self.tau);
        for evt in events {
            surface.update(evt.x, evt.y, evt.p.into(), evt.t);
        }
        surface
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    fn event(x: EventCoordinate, y: EventCoordinate, p: bool, t: EventTimestamp) -> EventCD {
        EventCD::new(x, y, crate::types::Polarity::from(p), t)
    }

    #[test]
    fn initializes_with_zero_timestamps_and_reports_geometry() {
        let surface = BaseTimeSurface::new(3, 2);

        assert_eq!(surface.width(), 3);
        assert_eq!(surface.height(), 2);
        assert_eq!(surface.get(0, 0, false), 0);
        assert_eq!(surface.get(2, 1, true), 0);
    }

    #[test]
    fn stores_timestamps_independently_by_coordinate_and_polarity() {
        let mut surface = BaseTimeSurface::new(3, 2);

        surface.update(1, 1, false, 100);
        surface.update(1, 1, true, 200);
        surface.update(2, 1, false, 300);

        assert_eq!(surface.get(1, 1, false), 100);
        assert_eq!(surface.get(1, 1, true), 200);
        assert_eq!(surface.get(2, 1, false), 300);
        assert_eq!(surface.get(2, 1, true), 0);
    }

    #[test]
    fn linear_decay_update_advances_existing_values_and_records_timestamp() {
        let mut surface = LinearDecayTimeSurface::new(2, 1, 10);

        surface.update(0, 0, true, 100);
        surface.update(1, 0, true, 105);
        surface.update(0, 0, false, 110);

        assert_eq!(surface.width(), 2);
        assert_eq!(surface.height(), 1);
        assert_close(surface.get(0, 0, true), 0.0);
        assert_close(surface.get(1, 0, true), 0.5);
        assert_close(surface.get(0, 0, false), 1.0);
        assert_eq!(surface.timestamp(0, 0, true), 100);
        assert_eq!(surface.timestamp(1, 0, true), 105);
        assert_eq!(surface.timestamp(0, 0, false), 110);
    }

    #[test]
    fn linear_decay_to_advances_without_setting_a_new_event() {
        let mut surface = LinearDecayTimeSurface::new(1, 1, 20);

        surface.update(0, 0, true, 100);
        surface.decay_to(105);
        surface.decay_to(115);

        assert_close(surface.get(0, 0, true), 0.25);
        assert_eq!(surface.timestamp(0, 0, true), 100);
    }

    #[test]
    fn exponential_decay_update_advances_existing_values_and_records_timestamp() {
        let mut surface = ExponentialDecayTimeSurface::new(2, 1, 10);

        surface.update(0, 0, true, 100);
        surface.update(1, 0, true, 110);

        assert_eq!(surface.width(), 2);
        assert_eq!(surface.height(), 1);
        assert_close(surface.get(0, 0, true), (-1.0f64).exp());
        assert_close(surface.get(1, 0, true), 1.0);
        assert_eq!(surface.timestamp(0, 0, true), 100);
        assert_eq!(surface.timestamp(1, 0, true), 110);
    }

    #[test]
    fn exponential_decay_to_advances_without_setting_a_new_event() {
        let mut surface = ExponentialDecayTimeSurface::new(1, 1, 10);

        surface.update(0, 0, true, 100);
        surface.decay_to(105);
        surface.decay_to(110);

        assert_close(surface.get(0, 0, true), (-1.0f64).exp());
        assert_eq!(surface.timestamp(0, 0, true), 100);
    }

    #[test]
    fn base_time_surface_transformer_builds_surface_from_events() {
        let transformer = BaseTimeSurfaceTransformer::new(2, 1);
        let events = vec![event(0, 0, true, 100), event(1, 0, false, 200)];

        let surface = transformer.transform_events(Box::new(events.into_iter()));

        assert_eq!(surface.get(0, 0, true), 100);
        assert_eq!(surface.get(1, 0, false), 200);
        assert_eq!(surface.get(1, 0, true), 0);
    }

    #[test]
    fn linear_decay_transformer_builds_decayed_surface_from_events() {
        let transformer = LinearDecayTimeSurfaceTransformer::new(2, 1, 10);
        let events = vec![event(0, 0, true, 100), event(1, 0, true, 105)];

        let surface = transformer.transform_events(Box::new(events.into_iter()));

        assert_close(surface.get(0, 0, true), 0.5);
        assert_close(surface.get(1, 0, true), 1.0);
    }

    #[test]
    fn exponential_decay_transformer_builds_decayed_surface_from_events() {
        let transformer = ExponentialDecayTimeSurfaceTransformer::new(2, 1, 10);
        let events = vec![event(0, 0, true, 100), event(1, 0, true, 110)];

        let surface = transformer.transform_events(Box::new(events.into_iter()));

        assert_close(surface.get(0, 0, true), (-1.0f64).exp());
        assert_close(surface.get(1, 0, true), 1.0);
    }

    #[test]
    fn base_time_surface_to_tensor_uses_max_for_single_channel() {
        let mut surface = BaseTimeSurface::new(2, 1);
        surface.update(0, 0, true, 100);
        surface.update(0, 0, false, 50);
        surface.update(1, 0, true, 200);

        let dual = surface.to_tensor(ChannelMode::Dual);
        assert_eq!(dual.at(0, 0, 0), 100.0);
        assert_eq!(dual.at(1, 0, 0), 50.0);
        assert_eq!(dual.at(0, 1, 0), 200.0);
        assert_eq!(dual.at(1, 1, 0), 0.0);

        let single = surface.to_tensor(ChannelMode::Single);
        assert_eq!(single.at(0, 0, 0), 100.0);
        assert_eq!(single.at(0, 1, 0), 200.0);
    }

    #[test]
    fn linear_decay_to_tensor_sums_polarities_in_single_channel() {
        let mut surface = LinearDecayTimeSurface::new(1, 1, 100);
        surface.update(0, 0, true, 100);
        surface.update(0, 0, false, 100);

        let dual = surface.to_tensor(ChannelMode::Dual);
        assert_close(dual.at(0, 0, 0), 1.0);
        assert_close(dual.at(1, 0, 0), 1.0);

        let single = surface.to_tensor(ChannelMode::Single);
        assert_close(single.at(0, 0, 0), 2.0);
    }

    #[test]
    fn exponential_decay_to_tensor_sums_polarities_in_single_channel() {
        let mut surface = ExponentialDecayTimeSurface::new(1, 1, 10);
        surface.update(0, 0, true, 100);
        surface.update(0, 0, false, 105);

        let dual = surface.to_tensor(ChannelMode::Dual);
        assert_close(dual.at(0, 0, 0), (-0.5f64).exp());
        assert_close(dual.at(1, 0, 0), 1.0);

        let single = surface.to_tensor(ChannelMode::Single);
        assert_close(single.at(0, 0, 0), (-0.5f64).exp() + 1.0);
    }
}
