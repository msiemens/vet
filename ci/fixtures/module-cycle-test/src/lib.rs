// `a`'s tests use `b`, and `b` uses `a`: a module cycle that exists only where `test` is
// set. `ci/check-module-cycles.sh --self-test` must report it; a check that leaves `test`
// off does not.
pub mod a {
    pub struct A;

    #[cfg(test)]
    mod tests {
        use crate::b::B;

        #[test]
        fn b_is_a_type_the_tests_know() {
            let _ = core::mem::size_of::<B>();
        }
    }
}

pub mod b {
    use crate::a::A;

    pub struct B(pub Option<Box<A>>);
}
