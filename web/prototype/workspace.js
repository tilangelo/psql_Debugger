const $ = (id) => document.getElementById(id);
const screens = {
  catalog: 'database-panel',
  constructor: 'constructor-view',
  debug: 'debug-view',
  editor: 'editor-view',
};
let wasActive = false;
function choose(next, remember = true) {
  if (!Object.hasOwnProperty.call(screens, next) || $('view-' + next).hidden || $('view-' + next).disabled)
    next = 'constructor';
  for (const [name, id] of Object.entries(screens)) {
    $(id).hidden = name !== next;
    $('view-' + name).setAttribute('aria-selected', String(name === next));
  }
  $('constructor-status').hidden = next !== 'constructor';
  if (remember && location.hash !== '#' + next) history.replaceState(null, '', '#' + next);
  document.dispatchEvent(new CustomEvent('workspaceview', { detail: { view: next } }));
}
for (const name of Object.keys(screens)) $('view-' + name).onclick = () => choose(name);
const tabs = Object.keys(screens).map((name) => $('view-' + name));
for (const tab of tabs)
  tab.onkeydown = (event) => {
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      const group = tabs.filter((item) => !item.hidden && !item.disabled);
      const next =
        event.key === 'Home'
          ? group[0]
          : event.key === 'End'
            ? group[group.length - 1]
            : group[(group.indexOf(tab) + (event.key === 'ArrowLeft' ? group.length - 1 : 1)) % group.length];
      next.click();
      next.focus();
    }
  };
document.addEventListener('catalogchange', (event) => {
  $('debug-empty').hidden = !!event.detail?.selected || wasActive;
});
document.addEventListener('debugstate', (event) => {
  const { active, phase } = event.detail;
  $('execution-state').textContent =
    {
      starting: 'Запуск вызова',
      running: 'Выполняется',
      paused: 'Вызов на паузе',
      stopping: 'Ожидаем отмену и откат',
    }[phase] || '';
  if (active) {
    $('debug-empty').hidden = true;
    if (!wasActive) choose('debug');
  }
  wasActive = active;
});
const token = document.querySelector('meta[name="pg-debugger-token"]')?.content;
$('view-debug').disabled = !token;
$('view-editor').disabled = !token;
window.addEventListener('hashchange', () => choose(location.hash.slice(1), false));
choose(token ? location.hash.slice(1) : 'constructor', false);

// Клик вне меню закрывает его.
document.addEventListener('click', (event) => {
  if (!(event.target instanceof Element)) return;
  const selected = event.target.closest('.compact-menu');
  for (const menu of document.querySelectorAll('.compact-menu[open]'))
    if (menu !== selected) menu.open = false;
});
document.addEventListener('keydown', (event) => {
  if (event.key !== 'Escape') return;
  const menu = event.target.closest?.('.compact-menu[open]');
  if (menu) {
    menu.open = false;
    menu.querySelector('summary').focus();
  }
});
