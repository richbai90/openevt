pub mod activity_noise_filter;
pub mod anti_flicker_filter;
pub mod histogram;
pub mod integral_image;
pub mod polarity_filter;
pub mod roi_filter;
pub mod spatio_temporal_contrast;
pub mod tensor;
pub mod time_surface;
pub mod trail_filter;

use crate::types::EventCD;

pub trait EvtTransformer {
    type Output;

    fn transform_events<'a>(&self, events: Box<dyn Iterator<Item = EventCD> + 'a>) -> Self::Output;
}

pub trait EvtProcessor {
    // Takes mutable processor state and an owned iterator, returning a boxed iterator.
    // The returned iterator's lifetime is bounded by both the input stream and self.
    fn process_events<'a>(
        &'a mut self,
        events: Box<dyn Iterator<Item = EventCD> + 'a>,
    ) -> Box<dyn Iterator<Item = EventCD> + 'a>;
}

pub struct Pipeline<'a> {
    processors: &'a mut [Box<dyn EvtProcessor>],
}

pub mod filters {
    pub use super::activity_noise_filter::ActivityNoiseFilter;
    pub use super::anti_flicker_filter::AntiFlickerFilter;
    pub use super::polarity_filter::PolarityFilter;
    pub use super::roi_filter::RoiFilter;
    pub use super::spatio_temporal_contrast::ActivityNoiseFilter as SpatioTemporalContrastFilter;
    pub use super::trail_filter::TrailFilter;
}

pub mod transformers {
    pub use super::histogram::HistogramTransformer;
    pub use super::integral_image::IntegralImageTransformer;
    pub use super::time_surface::{
        BaseTimeSurfaceTransformer, ExponentialDecayTimeSurfaceTransformer,
        LinearDecayTimeSurfaceTransformer,
    };
}

impl<'a> Pipeline<'a> {
    pub fn new(processors: &'a mut [Box<dyn EvtProcessor>]) -> Self {
        Self { processors }
    }

    pub fn process<'b>(
        &'b mut self,
        events: impl Iterator<Item = EventCD> + 'b,
    ) -> Box<dyn Iterator<Item = EventCD> + 'b>
    where
        'a: 'b, // The processors must outlive the evaluation lifetime 'b
    {
        let mut stream: Box<dyn Iterator<Item = EventCD> + 'b> = Box::new(events);
        for processor in &mut *self.processors {
            stream = processor.process_events(stream);
        }
        stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Polarity;

    struct PolarityGate(Polarity);

    impl EvtProcessor for PolarityGate {
        fn process_events<'a>(
            &'a mut self,
            events: Box<dyn Iterator<Item = EventCD> + 'a>,
        ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
            let polarity = self.0;
            Box::new(events.filter(move |evt| evt.p == polarity))
        }
    }

    struct MinTimestamp(u64);

    impl EvtProcessor for MinTimestamp {
        fn process_events<'a>(
            &'a mut self,
            events: Box<dyn Iterator<Item = EventCD> + 'a>,
        ) -> Box<dyn Iterator<Item = EventCD> + 'a> {
            let min_timestamp = self.0;
            Box::new(events.filter(move |evt| evt.t >= min_timestamp))
        }
    }

    #[test]
    fn pipeline_applies_processors_in_order() {
        let events = vec![
            EventCD::new(0, 0, Polarity::Negative, 30),
            EventCD::new(1, 0, Polarity::Positive, 10),
            EventCD::new(2, 0, Polarity::Positive, 20),
        ];
        let mut processors: Vec<Box<dyn EvtProcessor>> =
            vec![Box::new(PolarityGate(Polarity::Positive)), Box::new(MinTimestamp(15))];

        let mut pipeline = Pipeline::new(&mut processors);
        let output: Vec<_> = pipeline.process(events.clone().into_iter()).collect();

        assert_eq!(output, vec![events[2]]);
    }

    #[test]
    fn filters_module_reexports_filter_types() {
        let _: filters::ActivityNoiseFilter = filters::ActivityNoiseFilter::new(1, 1, 10);
        let _: filters::AntiFlickerFilter = filters::AntiFlickerFilter::new(1, 1, 3, 45, 55);
        let _: filters::PolarityFilter = filters::PolarityFilter::new(Polarity::Positive);
        let _: filters::RoiFilter = filters::RoiFilter::new(0, 2, 0, 2);
        let _: filters::SpatioTemporalContrastFilter =
            filters::SpatioTemporalContrastFilter::new(1, 1, 10, true);
        let _: filters::TrailFilter = filters::TrailFilter::new(1, 1, 10);
    }

    #[test]
    fn transformers_module_reexports_transformer_types() {
        let _: transformers::BaseTimeSurfaceTransformer =
            transformers::BaseTimeSurfaceTransformer::new(1, 1);
        let _: transformers::LinearDecayTimeSurfaceTransformer =
            transformers::LinearDecayTimeSurfaceTransformer::new(1, 1, 10);
        let _: transformers::ExponentialDecayTimeSurfaceTransformer =
            transformers::ExponentialDecayTimeSurfaceTransformer::new(1, 1, 10);
        let _: transformers::HistogramTransformer = transformers::HistogramTransformer::new(1, 1);
        let _: transformers::IntegralImageTransformer =
            transformers::IntegralImageTransformer::new(1, 1);
    }
}
