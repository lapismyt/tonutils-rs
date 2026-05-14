//! Tests for TL (Type Language) module

mod common;
mod golden;
mod requests;
mod responses;

use common::*;
use golden::*;
use requests::*;
use responses::*;
use tl_proto::{deserialize, serialize};
