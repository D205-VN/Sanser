import { mount } from 'svelte';
import AnnotationOverlay from './components/AnnotationOverlay.svelte';
import { setLocaleOverride } from './lib/i18n';
import './styles/tokens.css';
const target = document.getElementById('annotation-root');
if (!target) throw new Error('Annotation overlay root is missing');
// The overlay has no preferences access; the main window passes its language.
setLocaleOverride(new URLSearchParams(window.location.search).get('lang') === 'en' ? 'en' : 'vi');
mount(AnnotationOverlay, { target });
