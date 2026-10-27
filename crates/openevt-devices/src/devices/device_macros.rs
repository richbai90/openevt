#[macro_export]
/// Downcasts a facility guard to a concrete facility type.
macro_rules! facility_downcast {
    ($facility:expr, $target_type:ty) => {
        $facility
            .as_any()
            .downcast_ref::<$target_type>()
            .ok_or_else(|| {
                openevt_core::hal::facilities::FacilityError::FacilityDowncastError(
                    stringify!($facility).to_string(),
                    stringify!($target_type).to_string(),
                )
            })
    };
}

#[macro_export]
/// Mutably downcasts a facility guard to a concrete facility type.
macro_rules! facility_downcast_mut {
    ($facility:expr, $target_type:ty) => {
        $facility
            .as_any_mut()
            .downcast_mut::<$target_type>()
            .ok_or_else(|| {
                openevt_core::hal::facilities::FacilityError::FacilityDowncastError(
                    stringify!($facility).to_string(),
                    stringify!($target_type).to_string(),
                )
            })
    };
}
