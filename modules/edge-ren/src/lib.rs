#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Encode, Decode, HasCompact};
use frame_support::{Parameter, decl_error, decl_event, decl_module, decl_storage, ensure, traits::{Get, EnsureOrigin}};
use frame_system::{ensure_none, ensure_signed};
use edgeware_primitives::Balance;
use sp_core::ecdsa;
use sp_io::{crypto::secp256k1_ecdsa_recover, hashing::keccak_256};
use sp_runtime::{
	ModuleId,
	traits::{Member, StaticLookup, AccountIdConversion},
	transaction_validity::{
		InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity, ValidTransaction,
	},
	DispatchResult,
};
use sp_std::vec::Vec;
use orml_traits::MultiCurrency;

// mod mock;
// mod tests;

// const MODULE_ID: ModuleId = ModuleId(*b"edge-ren");

type EcdsaSignature = ecdsa::Signature;
type DestAddress = Vec<u8>;

type TokenIdOf<T> = <<T as Config>::Assets as MultiCurrency<<T as frame_system::Config>::AccountId>>::CurrencyId;
type BalanceOf<T> = <<T as Config>::Assets as MultiCurrency<<T as frame_system::Config>::AccountId>>::Balance;
// type BalanceOf<T> = u128;

pub trait Config: frame_system::Config {
	type Event: From<Event<Self>> + Into<<Self as frame_system::Config>::Event>;
	type RenVMTokenIdType: Member + Parameter + Default + Copy + HasCompact;
	type RenvmBridgeUnsignedPriority: Get<TransactionPriority>;
	type ControllerOrigin: EnsureOrigin<Self::Origin>; //Tmp config with EnsureRoot<AccountId>
	type ModuleId: Get<ModuleId>;
	type Assets: MultiCurrency<Self::AccountId>;
}

// struct RenTokenInfo
// ren_token_name String
// ren_token_asset_id how our assets pallets identifies this token, bounds same as the ones for asset
// ren_token_id What ren uses to identify this token on this chain (unique across chains and tokens)
// ren_token_pub_key The Pub key used to check the signature against.
// ren_token_proof proof of this token being registered on the RenVM, legitimizing and enabling stuff like recourse if burnAndRelease fails
// ren_token_mint_enabled,ren_token_burn_enabled to enable/disable currency, instead of delete; you probably do not want to overwrite a token anyway.
// ren_token_mint_fee, ren_token_burn_fee perentage fee on mint and burn
// ren_token_min_req min balance required below which assets will be lost and account may be removed

#[derive(Encode,Decode, Clone, PartialEq, Eq, Debug, Default)]
pub struct RenTokenInfo<RenVMTokenIdType, TokenIdOf, BalanceOf>//,RenTokenProofData>
	{
	ren_token_id: RenVMTokenIdType,
	ren_token_asset_id: TokenIdOf,
	ren_token_name: Vec<u8>, // TODO: Max length
	ren_token_renvm_id: [u8; 32],
	ren_token_pub_key: [u8; 20],
	// ren_token_proof: Vec<RenTokenProofData>,
	ren_token_mint_enabled: bool,
	ren_token_burn_enabled: bool,
	// ren_token_mint_fee: ,
	// ren_token_burn_fee: ,
	ren_token_min_req: BalanceOf,
}

type RenTokenInfoType<T> = RenTokenInfo<<T as Config>::RenVMTokenIdType, TokenIdOf<T>, BalanceOf<T>>;

decl_storage! {
	trait Store for Module<T: Config> as Template {
		/// Signature blacklist. This is required to prevent double claim.
		Signatures get(fn signatures): map hasher(opaque_twox_256) EcdsaSignature => Option<()>;
		/// Record burn event details
		BurnEvents get(fn burn_events): map hasher(twox_64_concat) u32 => Option<(T::BlockNumber, DestAddress, BalanceOf<T>)>;
		/// Next burn event ID
		NextBurnEventId get(fn next_burn_event_id): u32;

		RenTokenRegistry get(fn ren_token_registry): map hasher(blake2_128_concat) <T as Config>::RenVMTokenIdType => Option<RenTokenInfoType<T>>;
	}
}

decl_event!(
	pub enum Event<T> where
		<T as frame_system::Config>::AccountId,
		<T as Config>::RenVMTokenIdType,
		Balance = BalanceOf<T>
	{
		/// Asset minted. \[owner, amount\]
		Minted(AccountId, Balance),
		/// Asset burnt in this chain \[owner, dest, amount\]
		Burnt(AccountId, DestAddress, Balance),

		RenTokenAdded(RenVMTokenIdType),

		RenTokenUpdated(RenVMTokenIdType),

		RenTokenDeleted(RenVMTokenIdType),

		RenTokenSpent(RenVMTokenIdType, Balance),

		RenTokenMinted(AccountId, RenVMTokenIdType, Balance),

	}
);

decl_error! {
	pub enum Error for Module<T: Config> {
		/// The mint signature is invalid.
		InvalidMintSignature,
		/// The mint signature has already been used.
		SignatureAlreadyUsed,
		/// Burn ID overflow.
		BurnIdOverflow,
		/// The AssetId not found in pallet-asset Asset Storage map
		AssetIdDoesNotExist,
		/// The AssetId does not match RenVMBTCTokenId
		AssetIdDoesNotMatch,
		/// The funds aren't enough to burn the amount
		InsufficientFunds,
		/// RenTokenAlready Exists
		RenTokenAlreadyExists,
		/// No token with this ren_token_id found
		RenTokenNotFound,

		AssetIssueFailed,

		MintFailed,
	}
}

decl_module! {
	pub struct Module<T: Config> for enum Call where origin: T::Origin {
		type Error = Error<T>;

		fn deposit_event() = default;


		#[weight = 10_000]
		fn add_ren_token(
			origin,
			#[compact] _ren_token_id: T::RenVMTokenIdType,
			_ren_token_asset_id: TokenIdOf<T>,
			_ren_token_name: Vec<u8>,
			_ren_token_renvm_id: [u8; 32],
			_ren_token_pub_key: [u8; 20],
			_ren_token_mint_enabled: bool,
			_ren_token_burn_enabled: bool,
			_ren_token_min_req: BalanceOf<T>,
		) -> DispatchResult
		{
			T::ControllerOrigin::ensure_origin(origin)?;

			ensure!(!<RenTokenRegistry<T>>::contains_key(&_ren_token_id), Error::<T>::RenTokenAlreadyExists);

			// CREATE CALL

			// ?CHECK IF THE ASSET CAN BE CREATED BEFORE ATTEMPTING THIS: check if it already exists.
			// pallet_assets::Module::<T>::force_create(
			// 	RawOrigin::Root.into(),
			// 	_ren_token_asset_id.into(),
			// 	T::Lookup::unlookup(Self::account_id()),
			// 	u32::MAX,
			// 	_ren_token_min_req.into(),
			// ).or_else(|_|{Err(Error::<T>::AssetIssueFailed)})?;

			let _ren_token_info = RenTokenInfo{
				ren_token_id: _ren_token_id,
				ren_token_asset_id: _ren_token_asset_id,
				ren_token_name: _ren_token_name,
				ren_token_renvm_id: _ren_token_renvm_id,
				ren_token_pub_key: _ren_token_pub_key,
				ren_token_mint_enabled: _ren_token_mint_enabled,
				ren_token_burn_enabled: _ren_token_burn_enabled,
				ren_token_min_req: _ren_token_min_req,
			};


			RenTokenRegistry::<T>::insert(&_ren_token_id,_ren_token_info);

			Self::deposit_event(RawEvent::RenTokenAdded(_ren_token_id));
			Ok(())
		}

		#[weight = 10_000]
		fn update_ren_token(
			origin,
			#[compact] _ren_token_id: T::RenVMTokenIdType,
			_ren_token_asset_id_option: Option<TokenIdOf<T>>,
			_ren_token_name_option: Option<Vec<u8>>,
			_ren_token_renvm_id_option: Option<[u8; 32]>,
			_ren_token_pub_key_option: Option<[u8; 20]>,
			_ren_token_mint_enabled_option: Option<bool>,
			_ren_token_burn_enabled_option: Option<bool>,
			_ren_token_min_req_option: Option<BalanceOf<T>>,
		) -> DispatchResult
		{
			T::ControllerOrigin::ensure_origin(origin)?;

			RenTokenRegistry::<T>::try_mutate_exists(&_ren_token_id, |maybe_token_info| -> DispatchResult {
					let mut token_info = maybe_token_info.as_mut().ok_or(Error::<T>::RenTokenNotFound)?;

					if let Some(x) = _ren_token_asset_id_option { token_info.ren_token_asset_id = x; }
					if let Some(x) = _ren_token_name_option { token_info.ren_token_name = x; }
					if let Some(x) = _ren_token_renvm_id_option { token_info.ren_token_renvm_id = x; }
					if let Some(x) = _ren_token_pub_key_option { token_info.ren_token_pub_key = x; }
					if let Some(x) = _ren_token_mint_enabled_option { token_info.ren_token_mint_enabled = x; }
					if let Some(x) = _ren_token_burn_enabled_option { token_info.ren_token_burn_enabled = x; }
					if let Some(x) = _ren_token_min_req_option { token_info.ren_token_min_req = x; }

					Ok(())
				}

			)?;

			Self::deposit_event(RawEvent::RenTokenUpdated(_ren_token_id));
			Ok(())
		}


		#[weight = 10_000]
		fn delete_ren_token(
			origin,
			#[compact] _ren_token_id: T::RenVMTokenIdType,
		) -> DispatchResult
		{
			T::ControllerOrigin::ensure_origin(origin)?;

			ensure!(!<RenTokenRegistry<T>>::contains_key(&_ren_token_id), Error::<T>::RenTokenNotFound);

			// Attempt to destroy the asset
			// DESTROY CALL

			RenTokenRegistry::<T>::remove(&_ren_token_id);

			Self::deposit_event(RawEvent::RenTokenDeleted(_ren_token_id));
			Ok(())
		}

		#[weight = 10_000]
		fn spend_tokens(
			origin,
			#[compact] _ren_token_id: T::RenVMTokenIdType,
			who: T::AccountId,
			#[compact] amount: BalanceOf<T>,
		) -> DispatchResult
		{
			T::ControllerOrigin::ensure_origin(origin)?;
			ensure!(!<RenTokenRegistry<T>>::contains_key(&_ren_token_id), Error::<T>::RenTokenNotFound);

			//let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).map_or_else(|| Error::<T>::RenTokenNotFound, |_ren_token_info| _ren_token_info.ren_token_asset_id);
			let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).ok_or_else(|| Error::<T>::RenTokenNotFound)?.ren_token_asset_id;


			// TRANSFER CALL
			T::Assets::transfer(asset_id, &Self::account_id().into(), &who, amount)?;

			Self::deposit_event(RawEvent::RenTokenSpent(_ren_token_id, amount));
			Ok(())
		}


		#[weight = 10_000]
		fn mint(
			origin,
			who: T::AccountId,
			p_hash: [u8; 32],
			#[compact] amount: BalanceOf<T>, //BalanceOf<T>,
			n_hash: [u8; 32],
			sig: EcdsaSignature,
			#[compact] _ren_token_id: T::RenVMTokenIdType
		) -> DispatchResult
		{
			ensure_none(origin)?;

			//let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).map_or_else(|| Error::<T>::RenTokenNotFound, |_ren_token_info| _ren_token_info.ren_token_asset_id)?;
			let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).ok_or_else(|| Error::<T>::RenTokenNotFound)?.ren_token_asset_id;


			// MINT CALL
			T::Assets::deposit(asset_id, &who, amount.into())?;

			Signatures::insert(&sig, ());
			Self::deposit_event(RawEvent::RenTokenMinted(who, _ren_token_id, amount));
			Ok(())
		}

		#[weight = 10_000]
		fn burn(
			origin,
			#[compact] _ren_token_id: T::RenVMTokenIdType,
			to: DestAddress,
			#[compact] amount: BalanceOf<T>,
		) -> DispatchResult
		{
			let sender = ensure_signed(origin)?;
			//let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).map_or_else(|| Error::<T>::RenTokenNotFound, |_ren_token_info| _ren_token_info.ren_token_asset_id)?;
		 	let asset_id = RenTokenRegistry::<T>::get(&_ren_token_id).ok_or_else(|| Error::<T>::RenTokenNotFound)?.ren_token_asset_id;

			NextBurnEventId::try_mutate(|id| -> DispatchResult {
				let this_id = *id;
				*id = id.checked_add(1).ok_or(Error::<T>::BurnIdOverflow)?;

				// BURN CALL
				T::Assets::withdraw(asset_id, &sender, amount)?;

				BurnEvents::<T>::insert(this_id, (frame_system::Module::<T>::block_number(), &to, amount));
				Self::deposit_event(RawEvent::Burnt(sender, to, amount));

				Ok(())
			})?;
			Ok(())
		}

	}
}


impl<T: Config> Module<T> {

	/// The account ID that holds the pallet's accumulated funds on pallet-assets; mostly fees for now, maybe for loss of exsistential deposit later.
    pub fn account_id() -> T::AccountId {
        T::ModuleId::get().into_account()
    }

	// ABI-encode the values for creating the signature hash.
	fn signable_message(p_hash: &[u8; 32], amount: BalanceOf<T>, to: &[u8], n_hash: &[u8; 32], token: &[u8; 32]) -> Vec<u8> {

		let amount_slice = Encode::encode(&amount);

		// p_hash ++ amount ++ token ++ to ++ n_hash
		let length = 32 + 32 + 32 + 32 + 32;
		let mut v = Vec::with_capacity(length);
		v.extend_from_slice(&p_hash[..]);
		v.extend_from_slice(&[0u8; 16][..]);
		// v.extend_from_slice(&amount.to_be_bytes()[..]);
		v.extend_from_slice(&amount_slice);
		v.extend_from_slice(&token[..]);
		v.extend_from_slice(to);
		v.extend_from_slice(&n_hash[..]);
		v
	}

	// Verify that the signature has been signed by RenVM.
	fn verify_signature(
		p_hash: &[u8; 32],
		amount: BalanceOf<T>,
		to: &[u8],
		n_hash: &[u8; 32],
		sig: &[u8; 65],
		_ren_token_id: T::RenVMTokenIdType,
	) -> DispatchResult {
		// let identifier = RenTokenRegistry::<T>::get(&_ren_token_id).map_or_else(|| Error::<T>::RenTokenNotFound, |_ren_token_info| _ren_token_info.ren_token_renvm_id)?;
		let identifier = RenTokenRegistry::<T>::get(&_ren_token_id).ok_or_else(|| Error::<T>::RenTokenNotFound)?.ren_token_renvm_id;

		let signed_message_hash = keccak_256(&Self::signable_message(p_hash, amount, to, n_hash, &identifier));
		let recoverd =
			secp256k1_ecdsa_recover(&sig, &signed_message_hash).map_err(|_| Error::<T>::InvalidMintSignature)?;
		let addr = &keccak_256(&recoverd)[12..];

		let pub_key = RenTokenRegistry::<T>::get(&_ren_token_id).ok_or_else(|| Error::<T>::RenTokenNotFound)?.ren_token_pub_key;

		ensure!(addr == pub_key, Error::<T>::InvalidMintSignature);

		Ok(())
	}

}


#[allow(deprecated)]
impl<T: Config> frame_support::unsigned::ValidateUnsigned for Module<T> {
	type Call = Call<T>;

	fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
		if let Call::mint(who, p_hash, amount, n_hash, sig, _ren_token_id) = call {
			// check if already exists
			if Signatures::contains_key(&sig) {
				return InvalidTransaction::Stale.into();
			}

			let verify_result = Encode::using_encoded(&who, |encoded| -> DispatchResult {
				Self::verify_signature(&p_hash, *amount, encoded, &n_hash, &sig.0, *_ren_token_id)
			});

			// verify signature
			if verify_result.is_err() {
				return InvalidTransaction::BadProof.into();
			}

			ValidTransaction::with_tag_prefix("renvm-bridge")
				.priority(T::RenvmBridgeUnsignedPriority::get())
				.and_provides(sig)
				.longevity(64_u64)
				.propagate(true)
				.build()
		} else {
			InvalidTransaction::Call.into()
		}
	}
}


/// Simple ensure origin for the RenVM account
//
pub struct EnsureRenVM<T>(sp_std::marker::PhantomData<T>);
impl<T: Config> EnsureOrigin<T::Origin> for EnsureRenVM<T> {
	type Success = T::AccountId;
	fn try_origin(o: T::Origin) -> Result<Self::Success, T::Origin> {
		let renvm_id = Module::<T>::account_id();
		o.into().and_then(|o| match o {
			frame_system::RawOrigin::Signed(who) if who == renvm_id => Ok(renvm_id),
			r => Err(T::Origin::from(r)),
		})
	}

	// #[cfg(feature = "runtime-benchmarks")]
	fn successful_origin() -> T::Origin {
		T::Origin::from(frame_system::RawOrigin::Root)
	}

}