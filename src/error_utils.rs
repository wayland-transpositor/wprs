// Copyright 2024 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt;
use std::fmt::Display;

use anyhow::Context;

use crate::prelude::*;

// TODO(https://github.com/dtolnay/anyhow/issues/139): replace all this with the
// anyhow feature. That may not include the function name though.

// https://stackoverflow.com/questions/38088067/equivalent-of-func-or-function-in-rust
#[macro_export]
macro_rules! fname {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        type_name_of(f).strip_suffix("::f").unwrap()
    }};
}
pub use fname;

pub struct Location {
    pub fname: &'static str,
    pub file: &'static str,
    pub line: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} at {}:{}", self.fname, self.file, self.line)
    }
}

// TODO(https://github.com/rust-lang/rust/issues/95529): use panic::Location.
// That lets us stop having the caller call loc!() and passing it in. Location
// contains the absolute path of the file though, not the path relative to the
// project root, which is annoying. Maybe we truncate that?
#[macro_export]
macro_rules! loc {
    () => {
        Location {
            fname: fname!(),
            file: file!(),
            line: line!(),
        }
    };
}
pub use loc;

pub trait LocationContextExt<R, T, E>: Context<T, E> {
    fn with_context<C, F>(self, loc: Location, context: F) -> Result<T>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C;

    fn context<C>(self, loc: Location, context: C) -> Result<T>
    where
        C: Display + Send + Sync + 'static;

    fn location(self, loc: Location) -> Result<T>;
}

impl<R, T, E> LocationContextExt<R, T, E> for R
where
    R: Context<T, E>,
{
    fn with_context<C, F>(self, loc: Location, context: F) -> Result<T>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C,
    {
        Context::with_context(self, || format!("{}: {}", loc, context()))
    }

    fn context<C>(self, loc: Location, context: C) -> Result<T>
    where
        C: Display + Send + Sync + 'static,
    {
        LocationContextExt::with_context(self, loc, || context)
    }

    fn location(self, loc: Location) -> Result<T> {
        Context::with_context(self, || loc)
    }
}

/// Log a Result and then return it. Useful in cases such as `foo.try_into().log(loc!()).ok()`.
pub trait LogExt<T, E>: Context<T, E> {
    fn trace(self, loc: Location) -> Result<T>;
    fn debug(self, loc: Location) -> Result<T>;
    fn info(self, loc: Location) -> Result<T>;
    fn warn(self, loc: Location) -> Result<T>;
    fn error(self, loc: Location) -> Result<T>;
    fn log(self, loc: Location) -> Result<T>;
}

impl<R, T, E> LogExt<T, E> for R
where
    R: Context<T, E>,
{
    fn trace(self, loc: Location) -> Result<T> {
        let res = self.location(loc);
        if let Err(e) = &res {
            trace!("{e:?}");
        }
        res
    }

    fn debug(self, loc: Location) -> Result<T> {
        let res = self.location(loc);
        if let Err(e) = &res {
            debug!("{e:?}");
        }
        res
    }

    fn info(self, loc: Location) -> Result<T> {
        let res = self.location(loc);
        if let Err(e) = &res {
            info!("{e:?}");
        }
        res
    }

    fn warn(self, loc: Location) -> Result<T> {
        let res = self.location(loc);
        if let Err(e) = &res {
            warn!("{e:?}");
        }
        res
    }

    fn error(self, loc: Location) -> Result<T> {
        let res = self.location(loc);
        if let Err(e) = &res {
            error!("{e:?}");
        }
        res
    }

    fn log(self, loc: Location) -> Result<T> {
        self.error(loc)
    }
}

/// Useful when you can't return a Result because you're implementing a foreign
/// trait and you don't want to panic (or at least aren't yet sure if you want
/// to panic).
pub trait LogAndIgnoreExt<T, E>: LogExt<T, E> {
    fn trace_and_ignore(self, loc: Location);
    fn debug_and_ignore(self, loc: Location);
    fn info_and_ignore(self, loc: Location);
    fn warn_and_ignore(self, loc: Location);
    fn error_and_ignore(self, loc: Location);
    fn log_and_ignore(self, loc: Location);
}

impl<R, T, E> LogAndIgnoreExt<T, E> for R
where
    R: Context<T, E>,
{
    fn trace_and_ignore(self, loc: Location) {
        _ = self.trace(loc);
    }

    fn debug_and_ignore(self, loc: Location) {
        _ = self.debug(loc);
    }

    fn info_and_ignore(self, loc: Location) {
        _ = self.info(loc);
    }

    fn warn_and_ignore(self, loc: Location) {
        _ = self.warn(loc);
    }

    fn error_and_ignore(self, loc: Location) {
        _ = self.error(loc);
    }

    fn log_and_ignore(self, loc: Location) {
        _ = self.log(loc);
    }
}

/// Like ?, but for functions which return ().
#[macro_export]
macro_rules! log_and_return {
    ($expression:expr) => {
        match $expression {
            Ok(val) => val,
            Err(e) => {
                error!("{e:?}");
                return;
            },
        }
    };
}
pub use log_and_return;

/// Like log_and_return, but continues instead of returns.
#[macro_export]
macro_rules! log_and_continue {
    ($expression:expr) => {
        match $expression {
            Ok(val) => val,
            Err(e) => {
                error!("{e:?}");
                continue;
            },
        }
    };
}
pub use log_and_continue;

#[macro_export]
macro_rules! warn_and_return {
    ($expression:expr) => {
        match $expression {
            Ok(val) => val,
            Err(e) => {
                warn!("{e:?}");
                return;
            },
        }
    };
}
pub use warn_and_return;
