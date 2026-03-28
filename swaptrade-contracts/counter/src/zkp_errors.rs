/// Zero-Knowledge Proof Specific Errors
///
/// This module defines errors specific to ZKP operations

use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ZKPError {
    /// Proof verification failed
    ProofVerificationFailed = 500,
    /// Invalid proof format or structure
    InvalidProofFormat = 501,
    /// Proof has expired
    ProofExpired = 502,
    /// Commitment opening failed
    CommitmentOpeningFailed = 503,
    /// Range proof constraints violated
    RangeProofViolation = 504,
    /// Balance insufficient for transaction
    InsufficientPrivateBalance = 505,
    /// Transaction validity proof failed
    TransactionValidityFailed = 506,
    /// Unsupported ZKP scheme
    UnsupportedProofScheme = 507,
    /// Proof generation failed
    ProofGenerationFailed = 508,
    /// Circuit constraint satisfaction failed
    CircuitConstraintFailed = 509,
    /// Witness data is invalid
    InvalidWitness = 510,
    /// Maximum proofs per block exceeded
    ProofLimitExceeded = 511,
    /// Audit trail verification failed
    AuditTrailVerificationFailed = 512,
    /// Compliance check failed
    ComplianceCheckFailed = 513,
    /// Cryptographic operation failed
    CryptoOperationFailed = 514,
}
