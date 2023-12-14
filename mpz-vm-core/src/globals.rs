use crate::table::FunctionTable;

pub struct Globals<I, V> {
    pub func_table: FunctionTable<I, V>,
}

impl<I, V> Default for Globals<I, V> {
    fn default() -> Self {
        Self {
            func_table: Default::default(),
        }
    }
}
