//! Cheatcode trait and central dispatch table.

use revm::primitives::Bytes;

use crate::evm::cheatcode::effect::CheatcodeEffect;

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
        super::functions::warp::Warp::SELECTOR => dispatch::<super::functions::warp::Warp>(input),
        super::functions::roll::Roll::SELECTOR => dispatch::<super::functions::roll::Roll>(input),
        super::functions::fee::Fee::SELECTOR => dispatch::<super::functions::fee::Fee>(input),
        super::functions::coinbase::Coinbase::SELECTOR => {
            dispatch::<super::functions::coinbase::Coinbase>(input)
        }
        super::functions::prevrandao::Prevrandao::SELECTOR => {
            dispatch::<super::functions::prevrandao::Prevrandao>(input)
        }
        super::functions::chain_id::ChainId::SELECTOR => {
            dispatch::<super::functions::chain_id::ChainId>(input)
        }
        super::functions::difficulty::Difficulty::SELECTOR => {
            dispatch::<super::functions::difficulty::Difficulty>(input)
        }

        // Account manipulation
        super::functions::deal::Deal::SELECTOR => dispatch::<super::functions::deal::Deal>(input),
        super::functions::etch::Etch::SELECTOR => dispatch::<super::functions::etch::Etch>(input),
        super::functions::nonce::SetNonce::SELECTOR => {
            dispatch::<super::functions::nonce::SetNonce>(input)
        }
        super::functions::nonce::GetNonce::SELECTOR => {
            dispatch::<super::functions::nonce::GetNonce>(input)
        }
        super::functions::storage::Load::SELECTOR => {
            dispatch::<super::functions::storage::Load>(input)
        }
        super::functions::storage::Store::SELECTOR => {
            dispatch::<super::functions::storage::Store>(input)
        }

        // Prank
        super::functions::prank::Prank::SELECTOR => {
            dispatch::<super::functions::prank::Prank>(input)
        }
        super::functions::prank::PrankOrigin::SELECTOR => {
            dispatch::<super::functions::prank::PrankOrigin>(input)
        }
        super::functions::prank::StartPrank::SELECTOR => {
            dispatch::<super::functions::prank::StartPrank>(input)
        }
        super::functions::prank::StartPrankOrigin::SELECTOR => {
            dispatch::<super::functions::prank::StartPrankOrigin>(input)
        }
        super::functions::prank::StopPrank::SELECTOR => {
            dispatch::<super::functions::prank::StopPrank>(input)
        }

        // Label
        super::functions::label::Label::SELECTOR => {
            dispatch::<super::functions::label::Label>(input)
        }
        super::functions::label::GetLabel::SELECTOR => {
            dispatch::<super::functions::label::GetLabel>(input)
        }

        // String / type conversion
        super::functions::to_string::ToStringAddress::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringAddress>(input)
        }
        super::functions::to_string::ToStringBool::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringBool>(input)
        }
        super::functions::to_string::ToStringUint::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringUint>(input)
        }
        super::functions::to_string::ToStringInt::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringInt>(input)
        }
        super::functions::to_string::ToStringBytes32::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringBytes32>(input)
        }
        super::functions::to_string::ToStringBytes::SELECTOR => {
            dispatch::<super::functions::to_string::ToStringBytes>(input)
        }
        super::functions::parse::ParseUint::SELECTOR => {
            dispatch::<super::functions::parse::ParseUint>(input)
        }
        super::functions::parse::ParseInt::SELECTOR => {
            dispatch::<super::functions::parse::ParseInt>(input)
        }
        super::functions::parse::ParseBool::SELECTOR => {
            dispatch::<super::functions::parse::ParseBool>(input)
        }
        super::functions::parse::ParseAddress::SELECTOR => {
            dispatch::<super::functions::parse::ParseAddress>(input)
        }
        super::functions::parse::ParseBytes::SELECTOR => {
            dispatch::<super::functions::parse::ParseBytes>(input)
        }
        super::functions::parse::ParseBytes32::SELECTOR => {
            dispatch::<super::functions::parse::ParseBytes32>(input)
        }
        super::functions::get_code::GetCode::SELECTOR => {
            dispatch::<super::functions::get_code::GetCode>(input)
        }

        // Wallet / crypto
        super::functions::addr::Addr::SELECTOR => dispatch::<super::functions::addr::Addr>(input),
        super::functions::sign::Sign::SELECTOR => dispatch::<super::functions::sign::Sign>(input),

        // FFI
        super::functions::ffi::Ffi::SELECTOR => dispatch::<super::functions::ffi::Ffi>(input),

        // Unknown VM call: silently drop.
        _ => Some(vec![]),
    }
}
