import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { frontendErrorStore } from './lib/stores/frontendErrors.svelte';

const appElement = document.getElementById('app');

if (!appElement) {
  throw new Error('Could not find app root element');
}

// Installed before mount so a failure while the first view renders is recorded
// rather than only reaching the developer console.
frontendErrorStore.captureFrom(window);

const app = mount(App, {
  target: appElement,
});

export default app;
