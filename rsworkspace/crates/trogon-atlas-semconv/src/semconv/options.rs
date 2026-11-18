use super::scope::Scope;

#[derive(Clone, Debug, Default)]
pub struct InstrumentationOptions {
    pub scope: Scope,
}
