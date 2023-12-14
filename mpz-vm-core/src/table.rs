use crate::Function;

/// A functions id in a function table.
pub type FunctionId = u16;

pub struct FunctionTable<I, V> {
    functions: Vec<Function<I, V>>,
}

impl<I, V> Default for FunctionTable<I, V> {
    fn default() -> Self {
        Self {
            functions: Default::default(),
        }
    }
}

impl<I, V> FunctionTable<I, V> {
    /// Inserts a function into the table and returns its id.
    pub fn insert(&mut self, function: Function<I, V>) -> FunctionId {
        let id = self.functions.len() as FunctionId;
        self.functions.push(function);
        id
    }

    /// Returns a reference to the function with the given id.
    pub fn get(&self, id: FunctionId) -> Option<&Function<I, V>> {
        self.functions.get(id as usize)
    }
}
