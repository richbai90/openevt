use derive_new::new;

use crate::algorithms::EvtProcessor;
use crate::types::{EventCD, EventCoordinate};

#[derive(new)]
pub struct RoiFilter {
    x0: EventCoordinate,
    x1: EventCoordinate,
    y0: EventCoordinate,
    y1: EventCoordinate,
}

impl EvtProcessor for RoiFilter {
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
        Box::new(
            events.filter(|evt| {
                evt.x > self.x0 && evt.x < self.x1 && evt.y > self.y0 && evt.y < self.y1
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(x: EventCoordinate, y: EventCoordinate) -> EventCD {
        EventCD::new(x, y, crate::types::Polarity::Positive, x + y)
    }

    #[test]
    fn forwards_only_events_strictly_inside_roi_bounds() {
        let mut filter = RoiFilter::new(10, 20, 30, 40);
        let events = vec![
            event(10, 35),
            event(11, 31),
            event(20, 35),
            event(19, 39),
            event(19, 40),
            event(9, 35),
            event(15, 29),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[1], events[3]]);
    }
}
