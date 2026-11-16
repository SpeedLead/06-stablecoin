use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::Token2022,
    token_interface::{
        Mint, TokenAccount,
        mint_to, burn, transfer_checked,
        MintTo, Burn, TransferChecked,
    },
};
// anchor-spl 1.0 re-exports the Token-2022 interface crate as `spl_token_2022`
use anchor_spl::token_2022::spl_token_2022::{
    self,
    extension::ExtensionType,
    state::AccountState,
};

declare_id!("rYXfi25x9JMgau82aGMJMVUokq7JzueqehiJUmwR97Q");

#[program]
pub mod stablecoin {
    use super::*;

    /// Initialize the stablecoin mint and config with Token-2022 extensions
    /// Creates a mint with: DefaultAccountState, PermanentDelegate
    pub fn initialize(
        ctx: Context<Initialize>,
        _name: String,
        _symbol: String,
        _uri: String,
    ) -> Result<()> {
        // 1. Set up Config account
        let config = &mut ctx.accounts.config;
        config.admin = ctx.accounts.admin.key();
        config.bump = ctx.bumps.config;
        config.mint_bump = ctx.bumps.mint;
        config.mint = ctx.accounts.mint.key();
        config.paused = false;

        let mint_key = ctx.accounts.mint.key();
        let config_key = ctx.accounts.config.key();
        let token_program_id = ctx.accounts.token_program.key();

        // Signer seeds for mint PDA
        let mint_seeds: &[&[&[u8]]] = &[&[b"mint", &[ctx.bumps.mint]]];

        // 2. Calculate space for mint with extensions
        let extension_types = vec![
            ExtensionType::DefaultAccountState,
            ExtensionType::PermanentDelegate,
        ];

        // Get space for mint with extensions
        let total_space = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Mint>(
            &extension_types,
        )
        .unwrap();

        // 3. Create the mint account
        let lamports = Rent::get()?.minimum_balance(total_space);

        invoke_signed(
            &anchor_lang::solana_program::system_instruction::create_account(
                &ctx.accounts.admin.key(),
                &mint_key,
                lamports,
                total_space as u64,
                &token_program_id,
            ),
            &[
                ctx.accounts.admin.to_account_info(),
                ctx.accounts.mint.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            mint_seeds,
        )?;

        // 4. Initialize PermanentDelegate extension (Config PDA as delegate)
        invoke_signed(
            &spl_token_2022::instruction::initialize_permanent_delegate(
                &token_program_id,
                &mint_key,
                &config_key, // permanent delegate
            )?,
            &[ctx.accounts.mint.to_account_info()],
            mint_seeds,
        )?;

        // 5. Initialize DefaultAccountState extension (Initialized = unfrozen)
        invoke_signed(
            &spl_token_2022::extension::default_account_state::instruction::initialize_default_account_state(
                &token_program_id,
                &mint_key,
                &AccountState::Initialized, // Accounts start unfrozen
            )?,
            &[ctx.accounts.mint.to_account_info()],
            mint_seeds,
        )?;

        invoke_signed(
            &spl_token_2022::instruction::initialize_mint2(
                &token_program_id,
                &mint_key,
                &config_key,       // mint authority
                Some(&config_key), // freeze authority
                6,                 // decimals (USDC-style)
            )?,
            &[ctx.accounts.mint.to_account_info()],
            mint_seeds,
        )?;

        msg!("Initialized Token-2022 stablecoin with extensions");
        Ok(())
    }

    /// Configure a minter with a specific allowance
    /// Only the admin can call this instruction
    /// If the minter already exists, this updates their allowance
    pub fn configure_minter(ctx: Context<ConfigureMinter>, allowance: u64) -> Result<()> {
        let minter_config = &mut ctx.accounts.minter_config;

        // If not initialized, set the minter address
        if !minter_config.is_initialized {
            minter_config.minter = ctx.accounts.minter.key();
            minter_config.amount_minted = 0;
            minter_config.is_initialized = true;
            minter_config.bump = ctx.bumps.minter_config;
        }

        minter_config.allowance = allowance;

        msg!("Configured minter {} with allowance {}", ctx.accounts.minter.key(), allowance);

        Ok(())
    }

    /// Remove a minter's authorization
    /// Only the admin can call this instruction
    /// This closes the minter config account and returns rent to admin
    pub fn remove_minter(_ctx: Context<RemoveMinter>) -> Result<()> {
        msg!("Minter removed");
        Ok(())
    }

    /// Mint new stablecoins to a user
    /// Only authorized minters can call this instruction
    /// The minter must have sufficient allowance remaining
    pub fn mint_tokens(ctx: Context<MintTokens>, amount: u64) -> Result<()> {
        let config = &ctx.accounts.config;

        // Check not paused
        require!(!config.paused, StablecoinError::Paused);

        // Check and update minter allowance
        let minter_config = &mut ctx.accounts.minter_config;
        let remaining = minter_config.allowance.checked_sub(minter_config.amount_minted)
            .ok_or(StablecoinError::ExceedsAllowance)?;
        require!(amount <= remaining, StablecoinError::ExceedsAllowance);

        minter_config.amount_minted = minter_config.amount_minted.checked_add(amount)
            .ok_or(StablecoinError::Overflow)?;

        // Create the signer seeds for the mint authority PDA
        let signer_seeds: &[&[&[u8]]] = &[&[b"config", &[config.bump]]];

        // Mint tokens to the destination account
        mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.destination.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        msg!("Minted {} tokens to {}", amount, ctx.accounts.destination.key());

        Ok(())
    }

    /// Burn stablecoins from the caller's account
    /// Anyone can burn their own tokens
    /// In a real stablecoin, this would be called when users redeem for fiat
    pub fn burn_tokens(ctx: Context<BurnTokens>, amount: u64) -> Result<()> {
        burn(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            amount,
        )?;

        msg!("Burned {} tokens from {}", amount, ctx.accounts.token_account.key());

        Ok(())
    }

    /// Pause all minting operations
    /// Only the admin can call this instruction
    pub fn pause(ctx: Context<Pause>) -> Result<()> {
        ctx.accounts.config.paused = true;
        msg!("Stablecoin paused");
        Ok(())
    }

    /// Unpause minting operations
    /// Only the admin can call this instruction
    pub fn unpause(ctx: Context<Unpause>) -> Result<()> {
        ctx.accounts.config.paused = false;
        msg!("Stablecoin unpaused");
        Ok(())
    }

    /// Force burn tokens from any account using Permanent Delegate
    /// Only the admin can call this instruction - used for compliance
    pub fn force_burn(ctx: Context<ForceBurn>, amount: u64) -> Result<()> {
        let config = &ctx.accounts.config;
        let config_seeds: &[&[&[u8]]] = &[&[b"config", &[config.bump]]];

        // Use the permanent delegate authority to burn from any account
        burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                config_seeds,
            ),
            amount,
        )?;

        msg!("Force burned {} tokens from {}", amount, ctx.accounts.token_account.key());
        Ok(())
    }

    /// Force transfer tokens from any account using Permanent Delegate
    /// Only the admin can call this instruction - used for compliance
    pub fn force_transfer(ctx: Context<ForceTransfer>, amount: u64) -> Result<()> {
        let config = &ctx.accounts.config;
        let config_seeds: &[&[&[u8]]] = &[&[b"config", &[config.bump]]];

        // Use the permanent delegate authority to transfer from any account
        transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.from_token_account.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.to_token_account.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                config_seeds,
            ),
            amount,
            ctx.accounts.mint.decimals,
        )?;

        msg!("Force transferred {} tokens from {} to {}",
            amount,
            ctx.accounts.from_token_account.key(),
            ctx.accounts.to_token_account.key()
        );
        Ok(())
    }
}

// ============================================================================
// Account Structures
// ============================================================================

/// Config account that stores the stablecoin configuration
#[account]
#[derive(InitSpace)]
pub struct Config {
    /// The admin who can configure minters and blacklist accounts
    pub admin: Pubkey,      // 32 bytes
    /// The mint address of the stablecoin
    pub mint: Pubkey,       // 32 bytes
    /// Whether minting is paused
    pub paused: bool,       // 1 byte
    /// Bump seed for the config PDA
    pub bump: u8,           // 1 byte
    /// Bump seed for the mint PDA
    pub mint_bump: u8,      // 1 byte
}

/// Minter configuration account
/// Each authorized minter has their own config with an allowance
#[account]
#[derive(InitSpace)]
pub struct MinterConfig {
    /// The minter's public key
    pub minter: Pubkey,
    /// Maximum amount the minter can mint (total)
    pub allowance: u64,
    /// Amount already minted by this minter
    pub amount_minted: u64,
    /// Whether this account has been initialized
    pub is_initialized: bool,
    /// Bump seed for this PDA
    pub bump: u8,
}

// ============================================================================
// Instruction Contexts
// ============================================================================

#[derive(Accounts)]
#[instruction(name: String, symbol: String, uri: String)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    /// The config account that stores stablecoin settings
    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, Config>,

    /// The stablecoin mint - Token-2022 with extensions
    /// CHECK: Created and initialized via CPI in the instruction
    #[account(
        mut,
        seeds = [b"mint"],
        bump
    )]
    pub mint: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ConfigureMinter<'info> {
    /// Only the admin can configure minters
    #[account(
        mut,
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The minter being configured
    /// CHECK: This can be any account that will be authorized to mint
    pub minter: UncheckedAccount<'info>,

    /// The minter's configuration account
    #[account(
        init_if_needed,
        payer = admin,
        space = 8 + MinterConfig::INIT_SPACE,
        seeds = [b"minter", minter.key().as_ref()],
        bump
    )]
    pub minter_config: Account<'info, MinterConfig>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RemoveMinter<'info> {
    /// Only the admin can remove minters
    #[account(
        mut,
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The minter being removed
    /// CHECK: This is the minter whose config is being closed
    pub minter: UncheckedAccount<'info>,

    /// The minter's configuration account to close
    #[account(
        mut,
        close = admin,
        seeds = [b"minter", minter.key().as_ref()],
        bump = minter_config.bump
    )]
    pub minter_config: Account<'info, MinterConfig>,
}

#[derive(Accounts)]
pub struct MintTokens<'info> {
    /// The minter calling this instruction
    #[account(mut)]
    pub minter: Signer<'info>,

    /// The config account
    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The minter's configuration - verifies they are authorized
    #[account(
        mut,
        seeds = [b"minter", minter.key().as_ref()],
        bump = minter_config.bump,
        constraint = minter_config.is_initialized @ StablecoinError::NotMinter
    )]
    pub minter_config: Account<'info, MinterConfig>,

    /// The stablecoin mint (Token-2022)
    #[account(
        mut,
        seeds = [b"mint"],
        bump = config.mint_bump
    )]
    pub mint: InterfaceAccount<'info, Mint>,

    /// The destination token account to mint to (Token-2022)
    #[account(
        init_if_needed,
        payer = minter,
        associated_token::mint = mint,
        associated_token::authority = destination_owner,
        associated_token::token_program = token_program,
    )]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: The owner of the destination token account
    pub destination_owner: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BurnTokens<'info> {
    /// The owner of the token account burning tokens
    pub owner: Signer<'info>,

    /// The config account
    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The stablecoin mint (Token-2022)
    #[account(
        mut,
        seeds = [b"mint"],
        bump = config.mint_bump
    )]
    pub mint: InterfaceAccount<'info, Mint>,

    /// The token account to burn from (Token-2022)
    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = owner,
        associated_token::token_program = token_program,
    )]
    pub token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct Pause<'info> {
    /// Only the admin can pause
    #[account(
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
pub struct Unpause<'info> {
    /// Only the admin can unpause
    #[account(
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
pub struct ForceBurn<'info> {
    /// Only the admin can force burn
    #[account(
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The stablecoin mint (Token-2022)
    #[account(
        mut,
        seeds = [b"mint"],
        bump = config.mint_bump
    )]
    pub mint: InterfaceAccount<'info, Mint>,

    /// The token account to burn from (can be any account due to Permanent Delegate)
    #[account(
        mut,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

#[derive(Accounts)]
pub struct ForceTransfer<'info> {
    /// Only the admin can force transfer
    #[account(
        constraint = admin.key() == config.admin @ StablecoinError::Unauthorized
    )]
    pub admin: Signer<'info>,

    #[account(
        seeds = [b"config"],
        bump = config.bump
    )]
    pub config: Account<'info, Config>,

    /// The stablecoin mint (Token-2022)
    #[account(
        seeds = [b"mint"],
        bump = config.mint_bump
    )]
    pub mint: InterfaceAccount<'info, Mint>,

    /// The token account to transfer from (can be any account due to Permanent Delegate)
    #[account(
        mut,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub from_token_account: InterfaceAccount<'info, TokenAccount>,

    /// The token account to transfer to
    #[account(
        mut,
        token::mint = mint,
        token::token_program = token_program,
    )]
    pub to_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Program<'info, Token2022>,
}

// ============================================================================
// Error Codes
// ============================================================================

#[error_code]
pub enum StablecoinError {
    #[msg("You are not authorized to perform this action")]
    Unauthorized,
    #[msg("Minting is currently paused")]
    Paused,
    #[msg("Mint amount exceeds minter's remaining allowance")]
    ExceedsAllowance,
    #[msg("Account is not an authorized minter")]
    NotMinter,
    #[msg("Arithmetic overflow")]
    Overflow,
}
