#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FileStatusAnnotation {
    /// The entity is work-in-progress and subject to breaking changes.
    #[prost(bool, tag = "1")]
    pub work_in_progress: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct MessageStatusAnnotation {
    /// The entity is work-in-progress and subject to breaking changes.
    #[prost(bool, tag = "1")]
    pub work_in_progress: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FieldStatusAnnotation {
    /// The entity is work-in-progress and subject to breaking changes.
    #[prost(bool, tag = "1")]
    pub work_in_progress: bool,
}
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct StatusAnnotation {
    /// The entity is work-in-progress and subject to breaking changes.
    #[prost(bool, tag = "1")]
    pub work_in_progress: bool,
    /// The entity belongs to a package with the given version status.
    #[prost(enumeration = "PackageVersionStatus", tag = "2")]
    pub package_version_status: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum PackageVersionStatus {
    /// Unknown package version status.
    Unknown = 0,
    /// This version of the package is frozen.
    Frozen = 1,
    /// This version of the package is the active development version.
    Active = 2,
    /// This version of the package is the candidate for the next major version. It
    /// is typically machine generated from the active development version.
    NextMajorVersionCandidate = 3,
}
