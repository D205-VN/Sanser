//! Neon `PostgreSQL` storage and safe legacy preference migration.

mod backend;
mod legacy;

pub use backend::{
    DatabaseTarget, PreferenceStore, SecretConnectionString, Storage, StorageBackendKind,
    StorageError, StorageOptions,
};
pub use legacy::{
    LEGACY_MIGRATION_VERSION, LegacyMigrationError, LegacyMigrationReport, LegacyMigrator,
};
