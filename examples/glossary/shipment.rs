pub struct Parcel {
    pub payment_accepted: bool,
    pub packing_complete: bool,
}

pub fn ready_to_ship(parcel: &Parcel) -> bool {
    parcel.payment_accepted && parcel.packing_complete
}
