use crate::algorithms::EvtProcessor;
use crate::types::EventCD;
use derive_new::new;

#[derive(new)]
pub struct PolarityFilter {
    polarity: bool,
}

impl EvtProcessor for PolarityFilter {
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
        Box::new(events.filter(|evt| evt.p == self.polarity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(x: u64, p: bool) -> EventCD {
        EventCD::new(x, 0, p, x)
    }

    #[test]
    fn forwards_only_matching_polarity_events() {
        let mut filter = PolarityFilter::new(true);
        let events = vec![
            event(0, false),
            event(1, true),
            event(2, false),
            event(3, true),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[1], events[3]]);
    }
}
