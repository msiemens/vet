// `a` and `b` use each other: a module cycle that `ci/check-module-cycles.sh --self-test`
// must report.
pub mod a {
    use crate::b::B;

    pub struct A(pub Option<Box<B>>);
}

pub mod b {
    use crate::a::A;

    pub struct B(pub Option<Box<A>>);
}
