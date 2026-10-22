use crate::{
    algorithms::{EvtProcessor, time_surface::BaseTimeSurface},
    types::{EventCD, EventCoordinate, EventTimestamp},
};

pub struct ActivityNoiseFilter {
    dt: EventTimestamp,
    surface: BaseTimeSurface,
    cut_trail: bool,
    width: usize,
    height: usize,
    // Store cut status per pixel coordinate (x, y)
    is_cut: Vec<bool>,
}

impl ActivityNoiseFilter {
    pub fn new(
        width: EventCoordinate,
        height: EventCoordinate,
        dt: EventTimestamp,
        cut_trail: bool,
    ) -> Self {
        let w = width as usize;
        let h = height as usize;
        Self {
            dt,
            surface: BaseTimeSurface::new(width, height),
            cut_trail,
            width: w,
            height: h,
            is_cut: vec![false; w * h],
        }
    }
    #[inline]
    fn get_pixel_index(&self, x: EventCoordinate, y: EventCoordinate) -> usize {
        debug_assert!((x as usize) < self.width && (y as usize) < self.height);
        (y as usize * self.width) + x as usize
    }
}

impl EvtProcessor for ActivityNoiseFilter {
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
        Box::new(events.filter(move |evt| {
            let px_idx = self.get_pixel_index(evt.x, evt.y);

            let prev_t_same_p = self.surface.get(evt.x, evt.y, evt.p);
            let prev_t_opp_p = self.surface.get(evt.x, evt.y, !evt.p);

            let delta_same_p = evt.t.saturating_sub(prev_t_same_p);
            let delta_opp_p = evt.t.saturating_sub(prev_t_opp_p);

            let mut forward = false;

            // 1. Check Same Polarity Match within dt window
            if prev_t_same_p > 0 && delta_same_p <= self.dt && !self.is_cut[px_idx] {
                forward = true;
                if self.cut_trail {
                    self.is_cut[px_idx] = true;
                }
            }
            // 2. Check Opposite Polarity Match within dt window
            else if prev_t_opp_p > 0 && delta_opp_p <= self.dt {
                forward = true;
                // Opposite polarity event clears the cut lock on this pixel
                self.is_cut[px_idx] = false;
            }

            // Always update the time surface for the pixel coordinate
            self.surface.update(evt.x, evt.y, evt.p, evt.t);

            forward
        }))
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
    fn forwards_repeated_same_polarity_activity_when_trail_cutting_is_disabled() {
        let mut filter = ActivityNoiseFilter::new(1, 1, 10, false);
        let events = vec![
            event(true, 100),
            event(true, 105),
            event(true, 109),
            event(false, 112),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[1], events[2], events[3]]);
    }

    #[test]
    fn cut_trail_rejects_repeated_same_polarity_until_opposite_polarity_activity() {
        let mut filter = ActivityNoiseFilter::new(1, 1, 10, true);
        let events = vec![
            event(true, 100),
            event(true, 105),
            event(true, 109),
            event(false, 112),
            event(true, 116),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[1], events[3], events[4]]);
    }
}
