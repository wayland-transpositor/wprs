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

/// Implementation of https://git.suckless.org/wmname/file/wmname.c.html to
/// avoid depending on external packages.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::AtomEnum;
use x11rb::protocol::xproto::PropMode;
use x11rb::wrapper::ConnectionExt;

use crate::prelude::*;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_SUPPORTING_WM_CHECK,
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

pub fn set_wmname(dpy_name: Option<&str>, name: &str) -> Result<()> {
    let (conn, screen_num) = x11rb::connect(dpy_name).location(loc!())?;
    let atoms = Atoms::new(&conn)
        .location(loc!())?
        .reply()
        .location(loc!())?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    conn.change_property32(
        PropMode::REPLACE,
        root,
        atoms._NET_SUPPORTING_WM_CHECK,
        AtomEnum::WINDOW,
        &[root],
    )
    .location(loc!())?;

    conn.change_property8(
        PropMode::REPLACE,
        root,
        atoms._NET_WM_NAME,
        atoms.UTF8_STRING,
        name.as_bytes(),
    )?;

    conn.flush().location(loc!())?;
    Ok(())
}
