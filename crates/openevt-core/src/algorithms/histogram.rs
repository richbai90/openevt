//! Per-pixel event-count histograms.
//!
//! A [`Histogram`] counts how many events fall on each pixel, tracked
//! separately for each polarity. It implements [`EvtTransformer`] via
//! [`HistogramTransformer`] and materializes a tensor through
//! [`Histogram::to_tensor`].

use crate::{
    algorithms::{
        EvtTransformer,
        tensor::{ChannelMode, Tensor, tensor_from_polarities},
    },
    types::{EventCD, EventCoordinate},
};

/// A per-pixel count of events, tracked separately for each polarity.
#[derive(Debug, Clone)]
pub struct Histogram {
    width: EventCoordinate,
    height: EventCoordinate,
    pos: Vec<u64>,
    neg: Vec<u64>,
}

impl Histogram {
    pub fn new(width: EventCoordinate, height: EventCoordinate) -> Self {
        let size = (width as usize) * (height as usize);
        Self {
            width,
            height,
            pos: vec![0; size],
            neg: vec![0; size],
        }
    }

    /// Records one event at `(x, y)` with the given polarity.
    pub fn add(&mut self, x: EventCoordinate, y: EventCoordinate, p: bool) {
        let index = self.get_index(x, y);
        if p {
            self.pos[index] += 1;
        } else {
            self.neg[index] += 1;
        }
    }

    /// Number of events recorded at `(x, y)` for the given polarity.
    pub fn count(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> u64 {
        let index = self.get_index(x, y);
        if p {
            self.pos[index]
        } else {
            self.neg[index]
        }
    }

    pub fn width(&self) -> EventCoordinate {
        self.width
    }

    pub fn height(&self) -> EventCoordinate {
        self.height
    }

    /// Materializes the per-pixel counts as a tensor.
    ///
    /// Single-channel mode combines both polarities by summing the two
    /// per-pixel counts.
    pub fn to_tensor(&self, mode: ChannelMode) -> Tensor {
        tensor_from_polarities(
            self.width,
            self.height,
            mode,
            |x, y| self.count(x, y, true) as f64,
            |x, y| self.count(x, y, false) as f64,
            |pos, neg| pos + neg,
        )
    }

    #[inline]
    fn get_index(&self, x: EventCoordinate, y: EventCoordinate) -> usize {
        (y as usize * self.width as usize) + x as usize
    }
}

/// Transformer that builds a [`Histogram`] from a stream of events.
pub struct HistogramTransformer {
    width: EventCoordinate,
    height: EventCoordinate,
}

impl HistogramTransformer {
    pub fn new(width: EventCoordinate, height: EventCoordinate) -> Self {
        Self { width, height }
    }
}

impl EvtTransformer for HistogramTransformer {
    type Output = Histogram;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output {
        let mut histogram = Histogram::new(self.width, self.height);
        for evt in events {
            histogram.add(evt.x, evt.y, evt.p);
        }
        histogram
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(x: EventCoordinate, y: EventCoordinate, p: bool, t: EventCoordinate) -> EventCD {
        EventCD::new(x, y, p, t)
    }

    #[test]
    fn counts_events_per_pixel_and_polarity() {
        let mut histogram = Histogram::new(2, 2);
        histogram.add(0, 0, true);
        histogram.add(0, 0, true);
        histogram.add(0, 0, false);
        histogram.add(1, 1, false);

        assert_eq!(histogram.count(0, 0, true), 2);
        assert_eq!(histogram.count(0, 0, false), 1);
        assert_eq!(histogram.count(1, 1, false), 1);
        assert_eq!(histogram.count(1, 0, true), 0);
    }

    #[test]
    fn transformer_builds_histogram_from_events() {
        let transformer = HistogramTransformer::new(2, 2);
        let events = vec![
            event(0, 0, true, 0),
            event(0, 0, true, 1),
            event(1, 1, false, 2),
        ];

        let histogram = transformer.transform_events(Box::new(events.into_iter()));

        assert_eq!(histogram.count(0, 0, true), 2);
        assert_eq!(histogram.count(1, 1, false), 1);
    }

    #[test]
    fn dual_channel_tensor_separates_polarities() {
        let mut histogram = Histogram::new(2, 1);
        histogram.add(0, 0, true);
        histogram.add(0, 0, true);
        histogram.add(1, 0, false);

        let dual = histogram.to_tensor(ChannelMode::Dual);

        assert_eq!(dual.at(0, 0, 0), 2.0);
        assert_eq!(dual.at(1, 0, 0), 0.0);
        assert_eq!(dual.at(0, 1, 0), 0.0);
        assert_eq!(dual.at(1, 1, 0), 1.0);
    }

    #[test]
    fn single_channel_tensor_sums_polarities() {
        let mut histogram = Histogram::new(2, 1);
        histogram.add(0, 0, true);
        histogram.add(0, 0, true);
        histogram.add(1, 0, false);

        let single = histogram.to_tensor(ChannelMode::Single);

        assert_eq!(single.at(0, 0, 0), 2.0);
        assert_eq!(single.at(0, 1, 0), 1.0);
    }
}
