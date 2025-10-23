//! Tests for liteclient module

use super::types::*;
use crate::tl::TlError;

#[test]
fn test_lite_error_tl_error() {
    let tl_err = TlError::UnexpectedEof;
    let lite_err = LiteError::TlError(tl_err);
    
    match lite_err {
        LiteError::TlError(_) => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}

#[test]
fn test_lite_error_unexpected_message() {
    let err = LiteError::UnexpectedMessage;
    
    match err {
        LiteError::UnexpectedMessage => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}

#[test]
fn test_lite_error_display() {
    let err = LiteError::UnexpectedMessage;
    let error_string = format!("{}", err);
    
    assert!(error_string.contains("Unexpected"));
}

#[test]
fn test_lite_error_debug() {
    let err = LiteError::UnexpectedMessage;
    let debug_string = format!("{:?}", err);
    
    assert!(debug_string.contains("UnexpectedMessage"));
}

#[test]
fn test_tl_error_unexpected_eof() {
    let err = TlError::UnexpectedEof;
    let debug_str = format!("{:?}", err);
    
    assert!(debug_str.contains("UnexpectedEof"));
}

#[test]
fn test_lite_error_from_adnl_error() {
    use crate::adnl::helper_types::AdnlError;
    
    let adnl_err = AdnlError::IntegrityError;
    let lite_err: LiteError = adnl_err.into();
    
    match lite_err {
        LiteError::AdnlError(_) => {
            // Success
        }
        _ => panic!("Wrong error variant"),
    }
}
