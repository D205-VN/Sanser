import { mount } from 'svelte';
import VoiceWindow from './components/VoiceWindow.svelte';
import { setLocaleOverride } from './lib/i18n';
import { loadNativePreferences } from './lib/platform';
import './styles/tokens.css';
const target = document.getElementById('voice-root');
if (!target) throw new Error('Voice window root is missing');
mount(VoiceWindow, { target });

// The voice window has no preferences store of its own; follow the saved language.
void loadNativePreferences()
  .then((saved) => setLocaleOverride(saved?.locale === 'en' ? 'en' : 'vi'))
  .catch(() => undefined);
