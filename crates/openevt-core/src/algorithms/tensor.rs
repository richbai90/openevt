//! Channel-first numeric tensors produced from event data.
//!
//! A [`Tensor`] is a `(C, H, W)` float grid where the channel axis is first.
//! Event modalities (time surfaces, histograms, integral images) store their
//! per-pixel state separately for each polarity and use
//! [`tensor_from_polarities`] to materialize a tensor in the requested
//! [`ChannelMode`].

use crate::types::EventCoordinate;

/// How a generated [`Tensor`] stores event polarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelMode {
    /// A single channel combining both polarities per pixel.
    #[default]
    Single,
    /// Two channels: positive polarity (channel 0) then negative polarity
    /// (channel 1).
    Dual,
}

impl ChannelMode {
    /// Number of channels this mode produces.
    pub fn channels(self) -> usize {
        match self {
            ChannelMode::Single => 1,
            ChannelMode::Dual => 2,
        }
    }
}

/// A channel-first `(C, H, W)` float tensor produced from event data.
///
/// Values are stored in C-order (channel, then row, then column) so the flat
/// buffer maps directly onto a NumPy array of shape `(C, H, W)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tensor {
    width: EventCoordinate,
    height: EventCoordinate,
    channels: EventCoordinate,
    values: Vec<f64>,
}

impl Tensor {
    fn new_with_values(
        width: EventCoordinate,
        height: EventCoordinate,
        channels: EventCoordinate,
        values: Vec<f64>,
    ) -> Self {
        Self {
            width,
            height,
            channels,
            values,
        }
    }

    /// Tensor width in pixels.
    pub fn width(&self) -> EventCoordinate {
        self.width
    }

    /// Tensor height in pixels.
    pub fn height(&self) -> EventCoordinate {
        self.height
    }

    /// Number of channels (1 for single, 2 for dual).
    pub fn channels(&self) -> EventCoordinate {
        self.channels
    }

    /// Whether the tensor stores both polarities.
    pub fn is_dual(&self) -> bool {
        self.channels == 2
    }

    /// Total number of scalar values in the tensor.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the tensor has no pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Returns the value at `(channel, x, y)`.
    pub fn at(&self, channel: usize, x: EventCoordinate, y: EventCoordinate) -> f64 {
        let w = self.width as usize;
        let h = self.height as usize;
        self.values[channel * (w * h) + (y as usize) * w + x as usize]
    }

    /// Returns the flat C-order buffer of values.
    pub fn as_values(&self) -> &[f64] {
        &self.values
    }
}

/// Builds a [`Tensor`] from per-pixel positive and negative fields.
///
/// In [`ChannelMode::Dual`] the positive field becomes channel 0 and the
/// negative field channel 1. In [`ChannelMode::Single`] the two fields are
/// merged per pixel with `combine`.
pub fn tensor_from_polarities(
    width: EventCoordinate,
    height: EventCoordinate,
    mode: ChannelMode,
    pos: impl Fn(EventCoordinate, EventCoordinate) -> f64,
    neg: impl Fn(EventCoordinate, EventCoordinate) -> f64,
    combine: impl Fn(f64, f64) -> f64,
) -> Tensor {
    let w = width as usize;
    let h = height as usize;
    let hw = w * h;
    let channels = mode.channels();
    let mut values = vec![0.0; hw * channels];

    for y in 0..height {
        for x in 0..width {
            let px = pos(x, y);
            let nx = neg(x, y);
            let base = (y as usize) * w + x as usize;
            match mode {
                ChannelMode::Single => values[base] = combine(px, nx),
                ChannelMode::Dual => {
                    values[base] = px;
                    values[hw + base] = nx;
                }
            }
        }
    }

    Tensor::new_with_values(width, height, channels as EventCoordinate, values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_dual_channel_tensor_from_polarities() {
        let tensor = tensor_from_polarities(
            2,
            1,
            ChannelMode::Dual,
            |x, y| (x + y * 2 + 1) as f64,
            |x, y| (x + y * 2 + 10) as f64,
            |_p, _n| 0.0,
        );

        assert_eq!(tensor.width(), 2);
        assert_eq!(tensor.height(), 1);
        assert_eq!(tensor.channels(), 2);
        assert!(tensor.is_dual());
        assert_eq!(tensor.len(), 4);

        // channel 0 = pos, channel 1 = neg, layout (C, H, W)
        assert_eq!(tensor.at(0, 0, 0), 1.0);
        assert_eq!(tensor.at(0, 1, 0), 2.0);
        assert_eq!(tensor.at(1, 0, 0), 10.0);
        assert_eq!(tensor.at(1, 1, 0), 11.0);
    }

    #[test]
    fn builds_single_channel_tensor_by_combining_polarities() {
        let tensor = tensor_from_polarities(
            2,
            2,
            ChannelMode::Single,
            |x, _y| x as f64 + 1.0,
            |_x, y| y as f64 + 10.0,
            |p, n| p + n,
        );

        assert_eq!(tensor.channels(), 1);
        assert!(!tensor.is_dual());
        assert_eq!(tensor.len(), 4);

        assert_eq!(tensor.at(0, 0, 0), 1.0 + 10.0);
        assert_eq!(tensor.at(0, 1, 0), 2.0 + 10.0);
        assert_eq!(tensor.at(0, 0, 1), 1.0 + 11.0);
        assert_eq!(tensor.at(0, 1, 1), 2.0 + 11.0);
    }
}
