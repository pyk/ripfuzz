//! Cheatcode trait and central dispatch table.

use revm::primitives::Bytes;

use crate::vm::effect::CheatcodeEffect;

// ---------------------------------------------------------------------------
//  Trait every cheatcode struct must implement.
// ---------------------------------------------------------------------------

pub trait Cheatcode {
    const SELECTOR: [u8; 4];
    type Args;

    fn decode(input: &Bytes) -> Option<Self::Args>;
    fn effects(args: Self::Args) -> Vec<CheatcodeEffect>;
}

fn dispatch<C: Cheatcode>(input: &Bytes) -> Option<Vec<CheatcodeEffect>> {
    let args = C::decode(input)?;
    Some(C::effects(args))
}

pub fn dispatch_effects(sel: [u8; 4], input: &Bytes) -> Option<Vec<CheatcodeEffect>> {
    match sel {
        // Block / state manipulation
        super::cheatcodes::warp::Warp::SELECTOR => dispatch::<super::cheatcodes::warp::Warp>(input),
        super::cheatcodes::roll::Roll::SELECTOR => dispatch::<super::cheatcodes::roll::Roll>(input),
        super::cheatcodes::fee::Fee::SELECTOR => dispatch::<super::cheatcodes::fee::Fee>(input),
        super::cheatcodes::coinbase::Coinbase::SELECTOR => {
            dispatch::<super::cheatcodes::coinbase::Coinbase>(input)
        }
        super::cheatcodes::prevrandao::Prevrandao::SELECTOR => {
            dispatch::<super::cheatcodes::prevrandao::Prevrandao>(input)
        }
        super::cheatcodes::chain_id::ChainId::SELECTOR => {
            dispatch::<super::cheatcodes::chain_id::ChainId>(input)
        }
        super::cheatcodes::difficulty::Difficulty::SELECTOR => {
            dispatch::<super::cheatcodes::difficulty::Difficulty>(input)
        }

        // Account manipulation
        super::cheatcodes::deal::Deal::SELECTOR => dispatch::<super::cheatcodes::deal::Deal>(input),
        super::cheatcodes::etch::Etch::SELECTOR => dispatch::<super::cheatcodes::etch::Etch>(input),
        super::cheatcodes::nonce::SetNonce::SELECTOR => {
            dispatch::<super::cheatcodes::nonce::SetNonce>(input)
        }
        super::cheatcodes::nonce::GetNonce::SELECTOR => {
            dispatch::<super::cheatcodes::nonce::GetNonce>(input)
        }
        super::cheatcodes::storage::Load::SELECTOR => {
            dispatch::<super::cheatcodes::storage::Load>(input)
        }
        super::cheatcodes::storage::Store::SELECTOR => {
            dispatch::<super::cheatcodes::storage::Store>(input)
        }

        // Prank
        super::cheatcodes::prank::Prank::SELECTOR => {
            dispatch::<super::cheatcodes::prank::Prank>(input)
        }
        super::cheatcodes::prank::PrankOrigin::SELECTOR => {
            dispatch::<super::cheatcodes::prank::PrankOrigin>(input)
        }
        super::cheatcodes::prank::StartPrank::SELECTOR => {
            dispatch::<super::cheatcodes::prank::StartPrank>(input)
        }
        super::cheatcodes::prank::StartPrankOrigin::SELECTOR => {
            dispatch::<super::cheatcodes::prank::StartPrankOrigin>(input)
        }
        super::cheatcodes::prank::StopPrank::SELECTOR => {
            dispatch::<super::cheatcodes::prank::StopPrank>(input)
        }

        // Label
        super::cheatcodes::label::Label::SELECTOR => {
            dispatch::<super::cheatcodes::label::Label>(input)
        }
        super::cheatcodes::label::GetLabel::SELECTOR => {
            dispatch::<super::cheatcodes::label::GetLabel>(input)
        }

        // String / type conversion
        super::cheatcodes::to_string::ToStringAddress::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringAddress>(input)
        }
        super::cheatcodes::to_string::ToStringBool::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringBool>(input)
        }
        super::cheatcodes::to_string::ToStringUint::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringUint>(input)
        }
        super::cheatcodes::to_string::ToStringInt::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringInt>(input)
        }
        super::cheatcodes::to_string::ToStringBytes32::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringBytes32>(input)
        }
        super::cheatcodes::to_string::ToStringBytes::SELECTOR => {
            dispatch::<super::cheatcodes::to_string::ToStringBytes>(input)
        }
        super::cheatcodes::parse::ParseUint::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseUint>(input)
        }
        super::cheatcodes::parse::ParseInt::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseInt>(input)
        }
        super::cheatcodes::parse::ParseBool::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseBool>(input)
        }
        super::cheatcodes::parse::ParseAddress::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseAddress>(input)
        }
        super::cheatcodes::parse::ParseBytes::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseBytes>(input)
        }
        super::cheatcodes::parse::ParseBytes32::SELECTOR => {
            dispatch::<super::cheatcodes::parse::ParseBytes32>(input)
        }
        super::cheatcodes::get_code::GetCode::SELECTOR => {
            dispatch::<super::cheatcodes::get_code::GetCode>(input)
        }

        // Wallet / crypto
        super::cheatcodes::addr::Addr::SELECTOR => dispatch::<super::cheatcodes::addr::Addr>(input),
        super::cheatcodes::sign::Sign::SELECTOR => dispatch::<super::cheatcodes::sign::Sign>(input),

        // FFI
        super::cheatcodes::ffi::Ffi::SELECTOR => dispatch::<super::cheatcodes::ffi::Ffi>(input),

        // Unknown VM call: silently drop.
        _ => Some(vec![]),
    }
}
