import { expect, it } from 'vitest';
import { groupComputers, isOlderRegistration } from './computers';
import type { Device } from './types';

const host = { id: 'host', computerId: 'one', name: 'Studio', platform: 'Windows', deviceRole: 'host', online: false, pinned: false } as Device;
const client = { ...host, id: 'client', deviceRole: 'client', online: true, pinned: true } as Device;

it('shows one computer, connects through its host, and trusts its client identity', () => {
  expect(groupComputers([client, host])).toMatchObject([{ id: 'host', requesterId: 'client', online: false, pinned: true, memberIds: ['client', 'host'] }]);
});
it('does not merge different computers with the same name or guess legacy identities', () => {
  expect(groupComputers([host, { ...host, id: 'other', computerId: 'two' }])).toHaveLength(2);
  expect(groupComputers([{ ...host, computerId: undefined }, { ...client, computerId: undefined }])).toHaveLength(2);
});
it('excludes both roles of the local computer and removes repeated page rows', () => {
  expect(groupComputers([host, client], ['client'])).toEqual([]);
  expect(groupComputers([host, host])[0]?.memberIds).toEqual(['host']);
});
it('keeps named or online computers visible and only folds unidentified old defaults', () => {
  expect(isOlderRegistration({ ...host, computerId: undefined, name: 'Sanser Host' })).toBe(true);
  expect(isOlderRegistration({ ...host, computerId: undefined })).toBe(false);
  expect(isOlderRegistration({ ...host, name: 'Sanser Host' })).toBe(false);
  expect(isOlderRegistration({ ...client, computerId: undefined, name: 'This Windows PC' })).toBe(false);
});
