import { mount } from 'svelte';
import VoiceWindow from './components/VoiceWindow.svelte';
import './styles/tokens.css';
const target = document.getElementById('voice-root');
if (!target) throw new Error('Voice window root is missing');
mount(VoiceWindow, { target });
