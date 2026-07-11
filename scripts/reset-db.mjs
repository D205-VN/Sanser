// Temporary script to reset the Neon database
// Usage: node scripts/reset-db.mjs

import { readFileSync } from 'fs';
import { resolve } from 'path';

// Read DATABASE_URL from .env
const envPath = resolve(import.meta.dirname, '..', '.env');
const envContent = readFileSync(envPath, 'utf-8');
const match = envContent.match(/^DATABASE_URL=(.+)$/m);
if (!match) {
  console.error('DATABASE_URL not found in .env');
  process.exit(1);
}

const databaseUrl = match[1].trim();

// Use pg module (from node_modules or npx)
const { default: pg } = await import('pg');
const client = new pg.Client({ connectionString: databaseUrl, ssl: { rejectUnauthorized: false } });

try {
  await client.connect();
  console.log('Connected to Neon database.');
  
  await client.query('DROP SCHEMA public CASCADE;');
  console.log('Dropped public schema.');
  
  await client.query('CREATE SCHEMA public;');
  console.log('Recreated public schema.');
  
  // Grant default permissions
  await client.query('GRANT ALL ON SCHEMA public TO neondb_owner;');
  await client.query('GRANT ALL ON SCHEMA public TO public;');
  console.log('Permissions restored.');
  
  console.log('\n✅ Database reset complete! Run "npm run server:dev" to re-run migrations.');
} catch (err) {
  console.error('Error:', err.message);
  process.exit(1);
} finally {
  await client.end();
}
