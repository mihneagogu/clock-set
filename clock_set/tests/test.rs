use clock_set;

#[cfg(test)]
mod tests {
    use super::*;
    use clock_set::CSet;
    #[test]
    fn empty_construction_doesnt_reformat_harddisk() {
        let set = CSet::<()>::new();
        assert_eq!(0, set.size());
    }
}
