/// What a highlighted range is, as far as the theme tells them apart. The numbers are the wire
/// encoding (see [`crate::encode`]); the UI's list of kinds is in the same order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    Keyword = 0,
    String = 1,
    /// Numbers, booleans, `null` and the like.
    Number = 2,
    Comment = 3,
    Type = 4,
    Function = 5,
    /// Attributes, annotations and markup tags.
    Attribute = 6,
    Heading = 7,
    Link = 8,
    /// The namespace, module or container part of a qualified name in a signature.
    Qualifier = 9,
}
