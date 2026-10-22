use crate::{
    algorithms::{EvtProcessor, time_surface::BaseTimeSurface},
    types::{EventCoordinate, EventTimestamp},
};

pub struct TrailFilter {
    dt: EventTimestamp,
    surface: BaseTimeSurface,
}

impl TrailFilter {
    pub fn new(width: EventCoordinate, height: EventCoordinate, dt: EventTimestamp) -> Self {
        Self {
            dt,
            surface: BaseTimeSurface::new(width, height),
        }
    }
}

impl EvtProcessor for TrailFilter {
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = crate::types::EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = crate::types::EventCD> + 'a> {
        Box::new(events.filter(move |evt| {
            let same_pol_t = self.surface.get(evt.x, evt.y, evt.p);
            let opp_pol_t = self.surface.get(evt.x, evt.y, !evt.p);

            let pol_change = opp_pol_t > same_pol_t;
            let time_elapsed = same_pol_t == 0 || evt.t.saturating_sub(same_pol_t) >= self.dt;

            let valid = pol_change || time_elapsed;

            if valid {
                self.surface.update(evt.x, evt.y, evt.p, evt.t);
            }

            valid
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        algorithms::EvtProcessor,
        types::{EventCD, EventTimestamp},
    };

    fn event(p: bool, t: EventTimestamp) -> EventCD {
        EventCD::new(0, 0, p, t)
    }

    #[test]
    fn rejects_same_polarity_trail_events_until_threshold_or_polarity_change() {
        let mut filter = TrailFilter::new(1, 1, 10);
        let events = vec![
            event(true, 100),
            event(true, 105),
            event(true, 109),
            event(true, 111),
            event(false, 112),
            event(true, 113),
            event(true, 116),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[0], events[3], events[4], events[5]]);
    }
}
