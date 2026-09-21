import '@testing-library/jest-dom/vitest';

// Use the jsdom origin's storage, including on Node versions with native Web Storage.
const testWindow = (globalThis as unknown as { jsdom: { window: Window } }).jsdom.window;
Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: testWindow.localStorage });
Object.defineProperty(globalThis, 'sessionStorage', { configurable: true, value: testWindow.sessionStorage });
