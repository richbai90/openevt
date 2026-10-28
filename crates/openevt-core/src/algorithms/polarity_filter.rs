use crate::algorithms::EvtProcessor;
use crate::types::{EventCD, Polarity};
use derive_new::new;

#[derive(new)]
pub struct PolarityFilter {
    polarity: Polarity,
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

    fn event(x: u64, p: Polarity) -> EventCD {
        EventCD::new(x, 0, p, x)
    }

    #[test]
    fn forwards_only_matching_polarity_events() {
        let mut filter = PolarityFilter::new(Polarity::Positive);
        let events = vec![
            event(0, Polarity::Negative),
            event(1, Polarity::Positive),
            event(2, Polarity::Negative),
            event(3, Polarity::Positive),
        ];

        let output: Vec<_> = filter
            .process_events(Box::new(events.clone().into_iter()))
            .collect();

        assert_eq!(output, vec![events[1], events[3]]);
    }
}
