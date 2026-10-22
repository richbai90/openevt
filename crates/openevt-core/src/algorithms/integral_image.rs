//! Integral (prefix-sum) images of per-pixel event counts.
//!
//! An [`IntegralImage`] is the 2D spatial integral of a per-pixel event count.
//! After integration, `value(x, y, p)` equals the number of events at or above
//! and to the left of `(x, y)` for the given polarity, so the count of any
//! axis-aligned rectangle is available in constant time via [`rect_sum`].

use crate::{
    algorithms::{
        EvtTransformer,
        tensor::{ChannelMode, Tensor, tensor_from_polarities},
    },
    types::{EventCD, EventCoordinate},
};

/// The 2D spatial integral (prefix sum) of per-pixel event counts.
#[derive(Debug, Clone)]
pub struct IntegralImage {
    width: EventCoordinate,
    height: EventCoordinate,
    pos: Vec<u64>,
    neg: Vec<u64>,
}

impl IntegralImage {
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
    ///
    /// Counts are accumulated per pixel; call [`integrate`](Self::integrate)
    /// once all events have been added to build the prefix sums.
    pub fn add(&mut self, x: EventCoordinate, y: EventCoordinate, p: bool) {
        let index = self.get_index(x, y);
        if p {
            self.pos[index] += 1;
        } else {
            self.neg[index] += 1;
        }
    }

    /// Converts the accumulated per-pixel counts into 2D prefix sums.
    pub fn integrate(&mut self) {
        let width = self.width as usize;
        let height = self.height as usize;
        integrate_channel(&mut self.pos, width, height);
        integrate_channel(&mut self.neg, width, height);
    }

    /// Integral value at `(x, y)` for the given polarity.
    pub fn value(&self, x: EventCoordinate, y: EventCoordinate, p: bool) -> u64 {
        let index = self.get_index(x, y);
        if p {
            self.pos[index]
        } else {
            self.neg[index]
        }
    }

    /// Number of events inside the inclusive rectangle
    /// `(x0, y0)`..=`(x1, y1)` for the given polarity.
    pub fn rect_sum(
        &self,
        x0: EventCoordinate,
        y0: EventCoordinate,
        x1: EventCoordinate,
        y1: EventCoordinate,
        p: bool,
    ) -> u64 {
        let mut sum = self.value(x1, y1, p);
        if x0 > 0 {
            sum -= self.value(x0 - 1, y1, p);
        }
        if y0 > 0 {
            sum -= self.value(x1, y0 - 1, p);
        }
        if x0 > 0 && y0 > 0 {
            sum += self.value(x0 - 1, y0 - 1, p);
        }
        sum
    }

    pub fn width(&self) -> EventCoordinate {
        self.width
    }

    pub fn height(&self) -> EventCoordinate {
        self.height
    }

    /// Materializes the integrated per-pixel counts as a tensor.
    ///
    /// Single-channel mode combines both polarities by summing the two
    /// per-pixel integral values.
    pub fn to_tensor(&self, mode: ChannelMode) -> Tensor {
        tensor_from_polarities(
            self.width,
            self.height,
            mode,
            |x, y| self.value(x, y, true) as f64,
            |x, y| self.value(x, y, false) as f64,
            |pos, neg| pos + neg,
        )
    }

    #[inline]
    fn get_index(&self, x: EventCoordinate, y: EventCoordinate) -> usize {
        (y as usize * self.width as usize) + x as usize
    }
}

/// Builds the 2D prefix sum of a single per-pixel channel buffer.
fn integrate_channel(channel: &mut [u64], width: usize, height: usize) {
    // Cumulative sum along each row (x axis).
    for y in 0..height {
        let row = y * width;
        for x in 1..width {
            channel[row + x] += channel[row + x - 1];
        }
    }
    // Cumulative sum down each column (y axis).
    for y in 1..height {
        for x in 0..width {
            channel[y * width + x] += channel[(y - 1) * width + x];
        }
    }
}

/// Transformer that builds an [`IntegralImage`] from a stream of events.
pub struct IntegralImageTransformer {
    width: EventCoordinate,
    height: EventCoordinate,
}

impl IntegralImageTransformer {
    pub fn new(width: EventCoordinate, height: EventCoordinate) -> Self {
        Self { width, height }
    }
}

impl EvtTransformer for IntegralImageTransformer {
    type Output = IntegralImage;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output {
        let mut image = IntegralImage::new(self.width, self.height);
        for evt in events {
            image.add(evt.x, evt.y, evt.p);
        }
        image.integrate();
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(x: EventCoordinate, y: EventCoordinate, p: bool, t: EventCoordinate) -> EventCD {
        EventCD::new(x, y, p, t)
    }

    #[test]
    fn integrate_builds_prefix_sums() {
        let mut image = IntegralImage::new(3, 2);
        image.add(0, 0, true);
        image.add(1, 0, true);
        image.add(2, 1, true);
        image.integrate();

        // pos prefix sums:
        // row 0: 1 2 2
        // row 1: 1 2 3
        assert_eq!(image.value(0, 0, true), 1);
        assert_eq!(image.value(1, 0, true), 2);
        assert_eq!(image.value(2, 0, true), 2);
        assert_eq!(image.value(0, 1, true), 1);
        assert_eq!(image.value(1, 1, true), 2);
        assert_eq!(image.value(2, 1, true), 3);
    }

    #[test]
    fn rect_sum_counts_events_in_a_region() {
        let mut image = IntegralImage::new(4, 4);
        for (x, y) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            image.add(x, y, true);
        }
        image.integrate();

        assert_eq!(image.rect_sum(1, 1, 2, 2, true), 4);
        assert_eq!(image.rect_sum(0, 0, 0, 0, true), 0);
        assert_eq!(image.rect_sum(0, 0, 3, 3, true), 4);
    }

    #[test]
    fn transformer_builds_integrated_image_from_events() {
        let transformer = IntegralImageTransformer::new(2, 2);
        let events = vec![
            event(0, 0, true, 0),
            event(0, 1, false, 1),
            event(1, 1, false, 2),
        ];

        let image = transformer.transform_events(Box::new(events.into_iter()));

        assert_eq!(image.value(0, 0, true), 1);
        assert_eq!(image.value(1, 1, false), 2);
        assert_eq!(image.rect_sum(0, 1, 1, 1, false), 2);
    }

    #[test]
    fn dual_channel_tensor_separates_integrated_polarities() {
        let mut image = IntegralImage::new(2, 1);
        image.add(0, 0, true);
        image.add(1, 0, false);
        image.integrate();

        let dual = image.to_tensor(ChannelMode::Dual);
        assert_eq!(dual.at(0, 0, 0), 1.0);
        assert_eq!(dual.at(0, 1, 0), 1.0);
        assert_eq!(dual.at(1, 0, 0), 0.0);
        assert_eq!(dual.at(1, 1, 0), 1.0);
    }

    #[test]
    fn single_channel_tensor_sums_integrated_polarities() {
        let mut image = IntegralImage::new(2, 1);
        image.add(0, 0, true);
        image.add(1, 0, false);
        image.add(1, 0, false);
        image.integrate();

        let single = image.to_tensor(ChannelMode::Single);
        assert_eq!(single.at(0, 0, 0), 1.0);
        assert_eq!(single.at(0, 1, 0), 3.0);
    }
}
