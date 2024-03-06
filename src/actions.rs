use std::{collections::HashMap, default, io::Write};

pub type Functor<T> = Box<dyn (Fn(&mut T) -> FunctorRes<T>) + 'static + Send>;

pub struct FunctorRes<T> {
    to_add: Vec<(String, Functor<T>)>,
    to_rem: Vec<String>,
}

impl<T> FunctorRes<T> {
    pub fn new() -> Self {
        Self {
            to_add: Vec::new(),
            to_rem: Vec::new(),
        }
    }
}

pub struct Actions<T>
where
    T: Write + 'static + Send,
{
    writer: T,
    map: HashMap<String, Functor<T>>,
}

impl<T> Actions<T>
where
    T: Write + 'static + Send,
{
    pub fn new(writer: T) -> Self {
        Actions {
            writer,
            map: HashMap::new(),
        }
    }

    pub fn apply(self, key: &str) -> Self {
        let Self {
            mut writer,
            mut map,
        } = self;
        if let Some(func) = map.get(key) {
            let FunctorRes { mut to_add, to_rem } = func(&mut writer);
            map.retain(|x, _| !to_rem.contains(x));
            map.extend(to_add.drain(..));
        }
        Self { writer, map }
    }

    pub fn add_box(self: &mut Self, key: impl Into<String>, func: Functor<T>) {
        self.map.insert(key.into(), func);
    }

    pub fn add(
        self: &mut Self,
        key: impl Into<String>,
        func: impl (Fn(&mut T) -> FunctorRes<T>) + 'static + Send,
    ) {
        self.map.insert(key.into(), Box::new(func));
    }
}
