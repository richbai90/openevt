use crate::{
    algorithms::{EvtProcessor, time_surface::BaseTimeSurface},
    types::{EventCD, EventCoordinate, EventTimestamp},
};

const MICROSECONDS_PER_SECOND: EventTimestamp = 1_000_000;

// Algorithm used to remove flickering events given a frequency interval.
pub struct AntiFlickerFilter {
    surface: BaseTimeSurface,
    min_measurements: u32,
    min_freq: u32,
    max_freq: u32,
    width: usize,
    sample_counts: Vec<u32>,
    sample_offsets: Vec<usize>,
    frequency_samples: Vec<u32>,
    is_blocked: Vec<bool>,
    median_scratch: Vec<u32>,
}

impl AntiFlickerFilter {
    pub fn new(
        width: EventCoordinate,
        height: EventCoordinate,
        min_measurements: u32,
        min_freq: u32,
        max_freq: u32,
    ) -> Self {
        let width_usize = width as usize;
        let height_usize = height as usize;
        let pixel_count = width_usize * height_usize;
        let min_measurements = min_measurements.max(1);
        let sample_capacity = min_measurements as usize;

        AntiFlickerFilter {
            surface: BaseTimeSurface::new(width, height),
            min_measurements,
            min_freq,
            max_freq,
            width: width_usize,
            sample_counts: vec![0; pixel_count],
            sample_offsets: vec![0; pixel_count],
            frequency_samples: vec![0; pixel_count * sample_capacity],
            is_blocked: vec![false; pixel_count],
            median_scratch: Vec::with_capacity(sample_capacity),
        }
    }

    #[inline]
    fn get_pixel_index(&self, x: EventCoordinate, y: EventCoordinate) -> usize {
        (y as usize * self.width) + x as usize
    }

    #[inline]
    fn sample_capacity(&self) -> usize {
        self.min_measurements as usize
    }

    #[inline]
    fn frequency_from_period(period_us: EventTimestamp) -> Option<u32> {
        if period_us == 0 {
            return None;
        }

        let freq = (MICROSECONDS_PER_SECOND + period_us / 2) / period_us;
        Some(freq.min(u32::MAX as EventTimestamp) as u32)
    }

    #[inline]
    fn frequency_in_window(&self, frequency: u32) -> bool {
        frequency >= self.min_freq && frequency <= self.max_freq
    }

    fn push_frequency_sample(&mut self, px_idx: usize, frequency: u32) {
        let capacity = self.sample_capacity();
        let sample_idx = px_idx * capacity + self.sample_offsets[px_idx];
        self.frequency_samples[sample_idx] = frequency;
        self.sample_offsets[px_idx] = (self.sample_offsets[px_idx] + 1) % capacity;
        self.sample_counts[px_idx] = (self.sample_counts[px_idx] + 1).min(self.min_measurements);
    }

    fn median_frequency(&mut self, px_idx: usize) -> Option<u32> {
        let count = self.sample_counts[px_idx] as usize;
        if count < self.sample_capacity() {
            return None;
        }

        let start = px_idx * self.sample_capacity();
        self.median_scratch.clear();
        self.median_scratch
            .extend_from_slice(&self.frequency_samples[start..start + count]);
        self.median_scratch.sort_unstable();

        let mid = count / 2;
        if count % 2 == 1 {
            Some(self.median_scratch[mid])
        } else {
            let lo = self.median_scratch[mid - 1];
            let hi = self.median_scratch[mid];
            Some(lo + (hi - lo) / 2)
        }
    }

    fn reset_frequency_samples(&mut self, px_idx: usize) {
        self.sample_counts[px_idx] = 0;
        self.sample_offsets[px_idx] = 0;
        self.is_blocked[px_idx] = false;
    }

    fn process_event(&mut self, evt: &EventCD) -> bool {
        let px_idx = self.get_pixel_index(evt.x, evt.y);
        let prev_same_t = self.surface.get(evt.x, evt.y, evt.p);
        let prev_opp_t = self.surface.get(evt.x, evt.y, !evt.p);

        let mut forward = true;
        let is_alternating = prev_same_t > 0 && prev_opp_t > prev_same_t && evt.t > prev_same_t;

        if is_alternating {
            let period_us = evt.t - prev_same_t;
            if let Some(frequency) = Self::frequency_from_period(period_us) {
                forward = !(self.is_blocked[px_idx] && self.frequency_in_window(frequency));
                self.push_frequency_sample(px_idx, frequency);
                self.is_blocked[px_idx] = self
                    .median_frequency(px_idx)
                    .is_some_and(|median| self.frequency_in_window(median));
            } else {
                self.reset_frequency_samples(px_idx);
            }
        } else if prev_same_t > prev_opp_t {
            self.reset_frequency_samples(px_idx);
        }

        self.surface.update(evt.x, evt.y, evt.p, evt.t);
        forward
    }
}

impl EvtProcessor for AntiFlickerFilter {
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
        Box::new(events.filter(move |evt| self.process_event(evt)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::EvtProcessor;

    fn event(p: bool, t: EventTimestamp) -> EventCD {
        EventCD::new(0, 0, p, t)
    }

    #[test]
    fn suppresses_periodic_oscillation_after_minimum_samples() {
        let mut filter = AntiFlickerFilter::new(1, 1, 3, 45, 55);
        let events = vec![
            event(true, 1_000),
            event(false, 11_000),
            event(true, 21_000),
            event(false, 31_000),
            event(true, 41_000),
            event(false, 51_000),
            event(true, 61_000),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, events[..5]);
    }

    #[test]
    fn passes_oscillation_outside_frequency_window() {
        let mut filter = AntiFlickerFilter::new(1, 1, 3, 45, 55);
        let events = vec![
            event(true, 1_000),
            event(false, 6_000),
            event(true, 11_000),
            event(false, 16_000),
            event(true, 21_000),
            event(false, 26_000),
            event(true, 31_000),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, events);
    }

    #[test]
    fn passes_events_that_leave_the_blocked_frequency_window() {
        let mut filter = AntiFlickerFilter::new(1, 1, 3, 45, 55);
        let events = vec![
            event(true, 1_000),
            event(false, 11_000),
            event(true, 21_000),
            event(false, 31_000),
            event(true, 41_000),
            event(false, 51_000),
            event(true, 100_000),
            event(false, 149_000),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(
            output,
            vec![
                events[0], events[1], events[2], events[3], events[4], events[6], events[7],
            ]
        );
    }
}
