import { callArgumentError, callArgumentHint, recordLiteral } from './data-types.js';
import { createPauseComparison, newerSnapshot, outcomeSummary } from './debug-presentation.js';
import { blockingReason, readiness } from './readiness.js';
import { requestDatabase } from './database-client.js';
import { bodyTarget } from './body-target.js';
import { routineRevision, workPresets } from './work-presets.js';
import { mountNestedPointPlan } from './nested-point-plan.js';
import { mountTriggerCall } from './trigger-call.js';
const token = document.querySelector('meta[name="pg-debugger-token"]')?.content;
if (token) {
  const panel = document.createElement('section');
  panel.id = 'debug-panel';
  panel.hidden = true;
  panel.innerHTML = `<div class="debug-heading"><h2 id="debug-title">Отладка</h2><div class="debug-header-actions"><span id="debug-phase" role="status"></span><button id="debug-open-scenarios" type="button">Сценарии</button><details class="compact-menu" id="debug-more"><summary>Ещё</summary><div class="compact-menu-items"><button id="debug-plan-nested" type="button">Добавить вложенную точку</button></div></details><button id="debug-open-readiness" type="button">Готовность</button></div></div><p id="debug-network" role="status" hidden></p><div id="debug-arguments" class="debug-arguments"></div><div class="debug-toolbar"><button id="debug-start">Начать отладку</button><button id="debug-next" disabled>Шаг без входа</button><button id="debug-into" disabled>Шаг внутрь</button><button id="debug-out" disabled>Шаг наружу</button><button id="debug-continue" disabled>Продолжить</button><button id="debug-stop" disabled>Остановить</button><button id="debug-clear">Снять точки</button><button id="debug-open-nested" type="button" hidden>Вложенные точки</button></div><p id="debug-message" role="status"></p><div id="debug-point-notice" role="status"></div><div id="debug-point-list" aria-label="Выбранные точки остановки"></div><div id="debug-result-strip" hidden><span id="debug-result-short" role="status"></span><button id="debug-result-details" type="button">Подробности</button></div><div class="debug-columns"><section><h3>Тело PL/pgSQL</h3><p id="debug-frame-context" class="hint"></p><div id="debug-source-navigation" class="source-navigation" hidden><button id="source-prev">Предыдущие строки</button><label>К строке <input id="source-goto" type="number" min="1" inputmode="numeric"></label><button id="source-go">Перейти</button><button id="source-next">Следующие строки</button><span id="source-range" role="status"></span></div><div id="debug-source" aria-label="Исходный код для отладки"></div></section><section id="debug-inspection" hidden><h3>Переменные</h3><table id="debug-vars"><thead><tr><th>Имя</th><th>Значение</th></tr></thead><tbody></tbody></table><h3>Стек вызовов</h3><div id="debug-stack" aria-label="Кадры стека вызовов"></div></section></div><details id="debug-stops" hidden><summary id="debug-stops-summary">Наблюдавшиеся остановки</summary><ol></ol></details>`;
  document.getElementById('debug-view').append(panel);
  panel.insertAdjacentHTML(
    'beforeend',
    `
    <dialog id="debug-readiness" aria-labelledby="debug-readiness-title" class="debug-dialog">
      <div class="debug-dialog-heading"><h2 id="debug-readiness-title">Готовность отладки</h2><form method="dialog"><button>Закрыть</button></form></div>
      <dl></dl>
    </dialog>
    <dialog id="debug-scenario-panel" aria-labelledby="debug-scenario-title" class="debug-dialog">
      <div class="debug-dialog-heading"><h2 id="debug-scenario-title">Сценарии</h2><form method="dialog"><button>Закрыть</button></form></div>
      <div id="debug-scenario-picker" class="preset-row" hidden>
        <label>Сохранённый сценарий<select id="debug-scenario"></select></label>
        <button id="debug-scenario-load" type="button">Загрузить</button><button id="debug-scenario-remove" type="button">Удалить</button>
      </div>
      <form id="debug-scenario-form" class="preset-row"><label>Название<input id="debug-scenario-name" maxlength="80" required></label>
        <button type="submit" id="debug-scenario-save" title="Аргументы и точки сохраняются в этом браузере">Сохранить</button>
      </form>
      <p id="debug-scenario-status" role="status"></p>
    </dialog>
    <dialog id="debug-nested-points" aria-labelledby="debug-nested-title" class="debug-dialog">
      <div class="debug-dialog-heading"><h2 id="debug-nested-title">Вложенные точки</h2><form method="dialog"><button>Закрыть</button></form></div>
      <div id="debug-nested-list" aria-label="Точки вложенных вызовов"></div>
    </dialog>`,
  );
  const report = document.createElement('dialog');
  report.id = 'debug-report';
  report.setAttribute('aria-labelledby', 'debug-report-title');
  report.innerHTML = `<div class="debug-dialog-heading"><h2 id="debug-report-title">Последний вызов</h2><form method="dialog"><button>Закрыть</button></form></div><p id="debug-report-target"></p><div id="debug-result-summary"></div><section id="debug-result-table" hidden><div class="debug-toolbar"><p id="debug-table-count"></p><button id="debug-copy-table" type="button">Копировать результат</button><span id="debug-copy-status" role="status"></span></div><div class="debug-table-scroll" tabindex="0" aria-label="Таблица результата"><table><thead></thead><tbody></tbody></table></div></section><pre id="debug-outcome"></pre><p id="debug-final-warning"></p>`;
  document.getElementById('debug-view').append(report);
  const $ = (id) => document.getElementById(id);
  const comparison = createPauseComparison();
  let stopPending = false,
    catalogGeneration = 0;
  function network(message = '') {
    $('debug-network').hidden = !message;
    $('debug-network').textContent = message;
  }

  let catalog = null,
    selected = null,
    draft = null,
    session = null,
    points = new Set(),
    pending = false,
    polling = false,
    databaseBusy = false,
    shownPause = null;
  let scenarios = [];
  const triggerCall = mountTriggerCall($('debug-arguments'), () => buttons());
  const nestedPlan = mountNestedPointPlan($('debug-scenario-panel'), {
    browse: (payload) => requestDatabase('/api/catalog', payload),
    resolve: (payload) => api('/api/debug/resolve', payload),
    onBusy: () => buttons(),
  });
  $('debug-result-details').onclick = () => report.showModal();
  $('debug-open-scenarios').onclick = () => $('debug-scenario-panel').showModal();
  $('debug-open-readiness').onclick = () => $('debug-readiness').showModal();
  $('debug-plan-nested').onclick = () => {
    $('debug-more').open = false;
    nestedPlan.open();
  };
  $('debug-open-nested').onclick = () => {
    if (active()) $('debug-nested-points').showModal();
    else nestedPlan.open();
  };
  function renderReadiness() {
    const list = $('debug-readiness').querySelector('dl');
    list.replaceChildren();
    for (const check of readiness(catalog)) {
      const title = document.createElement('dt'),
        description = document.createElement('dd');
      title.textContent = check.label;
      description.textContent = check.message;
      description.dataset.state = check.state;
      list.append(title, description);
    }
  }
  const active = () => session && !['finished', 'idle'].includes(session.phase);
  const stable = (value) =>
    JSON.stringify(value, (_, v) =>
      v && typeof v === 'object' && !Array.isArray(v)
        ? Object.keys(v)
            .sort()
            .reduce((o, k) => ((o[k] = v[k]), o), {})
        : v,
    );
  async function api(path, body) {
    if (path === '/api/debug/start') return requestDatabase(path, body);
    const controller = new AbortController(),
      timer = setTimeout(() => controller.abort(), 5000);
    try {
      const response = await fetch(path, {
        method: 'POST',
        signal: controller.signal,
        headers: { 'content-type': 'application/json', 'x-pg-debugger-token': token },
        body: JSON.stringify(body),
      });
      let data;
      try {
        data = await response.json();
      } catch {
        throw Error(`Запрос не выполнен (${response.status}).`);
      }
      if (!response.ok) throw Error(data.error || 'Запрос не выполнен.');
      return data;
    } catch (error) {
      if (error.name === 'AbortError')
        throw Error(
          'Сервер не ответил за 5 секунд. Команда автоматически не повторяется; дождитесь обновления состояния.',
        );
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }
  document.addEventListener('databaserequest', (event) => {
    databaseBusy = event.detail.busy;
    buttons();
  });
  function buttons() {
    const running = !!active(),
      paused = running && session.phase === 'paused',
      unavailable = pending || session?.frame_pending;
    $('debug-inspection').hidden = !paused;
    $('debug-arguments').hidden = running || !$('debug-arguments').childElementCount;
    $('debug-start').hidden = running;
    $('debug-result-strip').hidden = running || !$('debug-result-short').textContent;
    for (const id of ['debug-next', 'debug-into', 'debug-out', 'debug-continue', 'debug-stop'])
      $(id).hidden = !running;
    if (running && $('debug-scenario-panel').open) $('debug-scenario-panel').close();
    const hasNested = running
      ? session?.nested_breakpoints?.some((group) => group.lines.length)
      : nestedPlan.hasPoints();
    $('debug-open-nested').hidden = !hasNested;
    if ((!running || !hasNested) && $('debug-nested-points').open) $('debug-nested-points').close();
    const reason =
      pending || nestedPlan.busy()
        ? 'Дождитесь ответа на команду.'
        : databaseBusy
          ? 'Выполняется другой запрос к базе.'
          : running
            ? 'Сначала завершите текущий вызов.'
            : !selected
              ? 'Подключитесь и выберите объект для нового запуска.'
              : blockingReason(catalog) || triggerCall.reason();
    $('debug-start').disabled = !!reason;
    $('debug-start').title = reason || 'Вызвать выбранный объект и начать пошаговую отладку';
    for (const id of ['debug-next', 'debug-into', 'debug-continue']) {
      $(id).disabled = !!unavailable || !paused;
      $(id).title = unavailable
        ? 'Дождитесь ответа на команду.'
        : !paused
          ? 'Команда доступна только на паузе.'
          : id === 'debug-next'
            ? 'Выполнить текущий оператор целиком, без входа во вложенную функцию.'
            : id === 'debug-into'
              ? 'Войти во вложенную PL/pgSQL-функцию или процедуру, если выполнение её достигнет.'
              : 'Выполнять до следующей точки или завершения.';
    }
    const hasCaller = session?.stack?.some((frame) => frame.level === 1);
    $('debug-out').disabled = !!unavailable || !paused || !hasCaller;
    $('debug-out').title = unavailable
      ? 'Дождитесь ответа на команду.'
      : !paused
        ? 'Команда доступна только на паузе.'
        : !hasCaller
          ? 'У выполняемого кадра нет вызывающего кадра PL/pgSQL.'
          : 'Выполнить до возврата в вызывающий кадр или до точки остановки.';
    $('debug-stop').disabled = stopPending || !running || session.phase === 'stopping';
    $('debug-stop').title = running
      ? 'Запросить отмену и дождаться завершения вызова и отката.'
      : 'Активного вызова нет.';
    const shownPoints = pointsFor(sourceOid());
    $('debug-clear').hidden = !shownPoints.size;
    $('debug-clear').disabled = !!unavailable || !shownPoints.size || (running && !paused);
    $('debug-clear').title = !shownPoints.size
      ? 'В этом объекте нет точек.'
      : running && !paused
        ? 'Точки меняются на паузе.'
        : 'Снять точки в просматриваемом объекте.';
    $('debug-source')
      .querySelectorAll('button')
      .forEach((button) => {
        button.disabled = !!unavailable || (running && !paused);
      });
    panel.querySelectorAll('#debug-point-list button,#debug-nested-list button').forEach((button) => {
      button.disabled = !!unavailable || (running && !paused);
    });
    $('debug-stack')
      .querySelectorAll('button')
      .forEach((button) => {
        button.disabled = !!unavailable || !paused || button.getAttribute('aria-pressed') === 'true';
      });
    const scenarioBlocked = running || pending || databaseBusy || !selected;
    nestedPlan.setLocked(scenarioBlocked, running);
    triggerCall.setLocked(scenarioBlocked);
    const isTrigger = triggerCall.isTrigger();
    $('debug-open-scenarios').hidden = running || isTrigger;
    $('debug-more').hidden = running;
    if (running) $('debug-more').open = false;
    $('debug-open-scenarios').disabled = scenarioBlocked;
    $('debug-plan-nested').disabled = scenarioBlocked;
    $('debug-scenario-picker').hidden = isTrigger || !scenarios.length;
    $('debug-scenario').disabled = scenarioBlocked || !scenarios.length;
    $('debug-scenario-load').disabled = $('debug-scenario-remove').disabled =
      scenarioBlocked || !$('debug-scenario').value;
    $('debug-scenario-name').disabled = $('debug-scenario-save').disabled = scenarioBlocked;
    $('debug-arguments')
      .querySelectorAll('input,textarea,select')
      .forEach((input) => {
        const record = input.closest('.record-argument');
        const nullSelected = record?.querySelector('[data-null]').checked;
        const fieldNull = input.closest('.record-field')?.querySelector('[data-field-null]').checked;
        const argIndex = record?.dataset.argument ?? input.dataset.argument ?? input.dataset.null;
        const usesDefault = argIndex !== undefined && byIndex('default', argIndex)?.checked;
        const acceptsValue = argIndex === undefined || selected?.arguments[Number(argIndex)]?.supported;
        input.disabled =
          running ||
          pending ||
          !acceptsValue ||
          (usesDefault && !input.hasAttribute('data-null') && !input.hasAttribute('data-default')) ||
          (record &&
            nullSelected &&
            !input.hasAttribute('data-null') &&
            !input.hasAttribute('data-default')) ||
          (input.type !== 'checkbox' &&
            (fieldNull ||
              (!record && input.closest('label')?.querySelector('input[type=checkbox]')?.checked)));
      });
    $('debug-phase').hidden = !session;
    $('debug-phase').dataset.phase = session?.phase || 'idle';
    document.dispatchEvent(
      new CustomEvent('debugstate', { detail: { active: running, phase: session?.phase || 'idle' } }),
    );
  }
  async function request(action) {
    if (pending) return;
    pending = true;
    buttons();
    try {
      await action();
    } catch (error) {
      $('debug-message').textContent = error.message;
    } finally {
      pending = false;
      buttons();
    }
  }
  const byIndex = (field, index) => $('debug-arguments').querySelector(`[data-${field}="${index}"]`);
  function toggleArgument(index, other) {
    const alternative = byIndex(other, index);
    if (alternative) alternative.checked = false;
    validateArgument(index);
    buttons();
  }
  function defaultOption(index, container) {
    const option = document.createElement('span'),
      input = document.createElement('input');
    option.className = 'default-argument';
    input.type = 'checkbox';
    input.dataset.default = index;
    input.setAttribute('aria-label', 'По умолчанию');
    input.onchange = () => toggleArgument(index, 'null');
    option.onclick = (event) => {
      if (event.target !== input) {
        event.preventDefault();
        input.click();
      }
    };
    option.append(input, document.createTextNode(' По умолчанию'));
    container.append(option);
  }
  function recordFields(arg, index) {
    const record = document.createElement('fieldset');
    record.className = 'record-argument';
    record.dataset.argument = index;
    record.tabIndex = -1;
    const caption = document.createElement('legend');
    caption.textContent = `${arg.mode} ${arg.name || 'Аргумент ' + (index + 1)} (${arg.type})`;
    record.append(caption);
    function nullOption(attribute, container) {
      const label = document.createElement('label'),
        control = document.createElement('input');
      control.type = 'checkbox';
      control.setAttribute(attribute, attribute === 'data-null' ? String(index) : '');
      control.onchange = () =>
        attribute === 'data-null' ? toggleArgument(index, 'default') : (validateArgument(index), buttons());
      label.append(control, document.createTextNode(' NULL'));
      container.append(label);
    }
    nullOption('data-null', record);
    if (arg.has_default) defaultOption(index, record);
    for (const field of arg.fields || []) {
      const row = document.createElement('div'),
        label = document.createElement('label'),
        value = document.createElement('textarea');
      row.className = 'record-field';
      label.textContent = `${field.name} (${field.type_name})`;
      value.dataset.fieldValue = '';
      value.rows = 1;
      value.oninput = () => validateArgument(index);
      label.append(value);
      row.append(label);
      nullOption('data-field-null', row);
      record.append(row);
    }
    const error = document.createElement('span');
    error.id = `debug-argument-${index}-error`;
    error.className = 'field-error';
    error.setAttribute('aria-live', 'polite');
    record.setAttribute('aria-describedby', error.id);
    record.append(error);
    return record;
  }
  function argumentForm() {
    const root = $('debug-arguments');
    root.replaceChildren();
    for (const [index, arg] of selected.arguments.entries()) {
      if (!['IN', 'INOUT'].includes(arg.mode)) continue;
      if (arg.format === 'composite' && ['IN', 'INOUT'].includes(arg.mode)) {
        root.append(recordFields(arg, index));
        continue;
      }
      const label = document.createElement('label');
      const caption = document.createElement('span');
      caption.className = 'argument-caption';
      caption.textContent = `${arg.mode} ${arg.name || 'Аргумент ' + (index + 1)} (${arg.type})`;
      label.append(caption);
      const input = document.createElement({ json: 'textarea', enum: 'select' }[arg.format] || 'input');
      input.dataset.argument = index;
      if (arg.format === 'enum') {
        const placeholder = new Option('Выберите значение', '', true, true);
        placeholder.disabled = true;
        input.add(placeholder);
        for (const value of arg.enum_labels || [])
          input.add(new Option(value === '' ? '(пустая строка)' : value, value));
      }
      const nullInput = document.createElement('input');
      nullInput.type = 'checkbox';
      nullInput.dataset.null = index;
      nullInput.onchange = () => toggleArgument(index, 'default');
      const nullLabel = document.createElement('span');
      nullLabel.append(nullInput, document.createTextNode(' NULL'));
      const error = document.createElement('span');
      error.id = `debug-argument-${index}-error`;
      error.className = 'field-error';
      error.setAttribute('aria-live', 'polite');
      input.setAttribute('aria-describedby', error.id);
      input.placeholder = arg.type === 'text' ? 'Текст' : callArgumentHint(arg);
      input.oninput = () => validateArgument(index);
      label.append(input, nullLabel);
      if (arg.has_default) defaultOption(index, label);
      label.append(error);
      if (!arg.supported) {
        const note = document.createElement('span');
        note.textContent = arg.has_default
          ? 'Доступно только значение по умолчанию'
          : 'Входной тип пока не поддерживается';
        label.append(note);
      }
      root.append(label);
    }
  }
  function argumentValue(index, input) {
    if (byIndex('null', index).checked || byIndex('default', index)?.checked) return null;
    if (input.tagName !== 'FIELDSET') return input.value;
    return recordLiteral(
      [...input.querySelectorAll('.record-field')].map((row) =>
        row.querySelector('[data-field-null]').checked ? null : row.querySelector('[data-field-value]').value,
      ),
    );
  }
  function validateArgument(index) {
    const input = $('debug-arguments').querySelector(`[data-argument="${index}"]`);
    if (!input) return '';
    const isNull = byIndex('null', index).checked || byIndex('default', index)?.checked;
    const arg = selected.arguments[index];
    const error =
      !isNull && !arg.supported
        ? 'Выберите «По умолчанию».'
        : !isNull && input.tagName === 'SELECT' && input.selectedIndex < 1
          ? 'Выберите значение.'
          : callArgumentError(arg, argumentValue(index, input));
    input.setAttribute('aria-invalid', String(!!error));
    $(`debug-argument-${index}-error`).textContent = error;
    return error;
  }
  function values() {
    return selected.arguments.map((arg, i) =>
      ['IN', 'INOUT'].includes(arg.mode)
        ? argumentValue(i, $('debug-arguments').querySelector(`[data-argument="${i}"]`))
        : null,
    );
  }
  function scenarioStatus(message) {
    $('debug-scenario-status').textContent = message;
  }
  function renderScenarios(selectedName = '') {
    scenarios = [];
    scenarioStatus('');
    const target = bodyTarget(catalog),
      select = $('debug-scenario');
    try {
      if (target) scenarios = workPresets.scenarios(target);
    } catch (error) {
      scenarioStatus(error.message);
    }
    select.replaceChildren(new Option(scenarios.length ? 'Выберите сценарий' : 'Сценариев пока нет', ''));
    scenarios.forEach((scenario, index) =>
      select.add(new Option(scenario.name, String(index), false, scenario.name === selectedName)),
    );
    buttons();
  }
  function captureArguments() {
    return selected.arguments.map((_, index) => {
      const input = byIndex('argument', index);
      if (!input) return null;
      if (validateArgument(index)) throw Error('Исправьте отмеченные аргументы перед сохранением сценария.');
      if (byIndex('default', index)?.checked) return { mode: 'default' };
      if (byIndex('null', index).checked) return { mode: 'null' };
      const value =
        input.tagName === 'FIELDSET'
          ? [...input.querySelectorAll('.record-field')].map((row) =>
              row.querySelector('[data-field-null]').checked
                ? null
                : row.querySelector('[data-field-value]').value,
            )
          : input.value;
      return { mode: 'value', value };
    });
  }
  function restoreArguments(args) {
    if (args.length !== selected.arguments.length)
      throw Error('Набор аргументов сценария не совпадает с объектом.');
    const fields = args.map((state, index) => {
      const input = byIndex('argument', index),
        def = byIndex('default', index),
        nullInput = byIndex('null', index);
      if (!input && state === null) return null;
      if (input && state) {
        if (
          (state.mode === 'default' && def) ||
          (state.mode === 'null' && selected.arguments[index].supported)
        )
          return { input, def, nullInput, state };
        if (state.mode === 'value' && selected.arguments[index].supported) {
          if (input.tagName === 'FIELDSET' && Array.isArray(state.value)) {
            const rows = [...input.querySelectorAll('.record-field')];
            if (rows.length === state.value.length) return { input, def, nullInput, state, rows };
          } else if (input.tagName === 'SELECT' && typeof state.value === 'string') {
            const option = [...input.options].findIndex((option, i) => i > 0 && option.value === state.value);
            if (option > 0) return { input, def, nullInput, state, option };
          } else if (
            input.tagName !== 'FIELDSET' &&
            input.tagName !== 'SELECT' &&
            typeof state.value === 'string'
          )
            return { input, def, nullInput, state };
        }
      }
      throw Error('Сценарий содержит аргумент, который нельзя восстановить.');
    });
    for (const [index, field] of fields.entries()) {
      if (!field) continue;
      const { input, def, nullInput, state, rows, option } = field;
      if (def) def.checked = state.mode === 'default';
      nullInput.checked = state.mode === 'null';
      if (state.mode === 'value') {
        if (rows)
          rows.forEach((row, i) => {
            row.querySelector('[data-field-null]').checked = state.value[i] === null;
            row.querySelector('[data-field-value]').value = state.value[i] ?? '';
          });
        else if (option !== undefined) input.selectedIndex = option;
        else input.value = state.value;
      } else if (input.tagName === 'FIELDSET') {
        input.querySelectorAll('[data-field-value]').forEach((field) => {
          field.value = '';
        });
        input.querySelectorAll('[data-field-null]').forEach((field) => {
          field.checked = false;
        });
      } else if (input.tagName === 'SELECT') {
        input.selectedIndex = 0;
      } else {
        input.value = '';
      }
      validateArgument(index);
    }
  }
  function scenarioAction(action) {
    return request(async () => {
      try {
        await action();
      } catch (error) {
        scenarioStatus(error.message);
      }
    });
  }
  const chosenScenario = () => scenarios[$('debug-scenario').value];
  $('debug-scenario').onchange = () => {
    $('debug-scenario-name').value = chosenScenario()?.name || '';
    scenarioStatus('');
    buttons();
  };
  $('debug-scenario-form').onsubmit = (event) => {
    event.preventDefault();
    if ($('debug-scenario-save').disabled) return;
    scenarioAction(async () => {
      const target = bodyTarget(catalog),
        object = selected,
        name = $('debug-scenario-name').value.trim();
      if (!name) throw Error('Введите название сценария.');
      const args = captureArguments(),
        savedPoints = [...points];
      const revision = await routineRevision(object);
      if (object !== selected) return;
      workPresets.saveScenario({ target, name, revision, args, points: savedPoints });
      renderScenarios(name);
      scenarioStatus('Сценарий сохранён.');
    });
  };
  $('debug-scenario-load').onclick = () =>
    scenarioAction(async () => {
      const scenario = chosenScenario(),
        object = selected;
      if (!scenario || !object) return;
      const revision = await routineRevision(object);
      if (object !== selected) return;
      if (revision !== scenario.revision)
        throw Error('Определение или типы аргументов изменились. Создайте сценарий по актуальному объекту.');
      restoreArguments(scenario.args);
      session = null;
      shownPause = null;
      sourcePage = 0;
      points = new Set(scenario.points);
      $('debug-phase').textContent = 'Готово к запуску';
      $('debug-point-notice').textContent = '';
      $('debug-vars').querySelector('tbody').replaceChildren();
      $('debug-stack').replaceChildren();
      $('debug-stops').hidden = true;
      renderSource();
      graphPosition();
      scenarioStatus('Аргументы и точки загружены. Запустите отладку, когда будете готовы.');
    });
  $('debug-scenario-remove').onclick = () =>
    scenarioAction(() => {
      const scenario = chosenScenario();
      if (!scenario) return;
      workPresets.removeScenario(scenario);
      renderScenarios();
      $('debug-scenario-name').value = '';
      scenarioStatus('Сценарий удалён.');
    });
  function nestedFrame() {
    return session?.phase === 'paused' && !!session.frame && session.frame.oid !== session.routine?.oid;
  }
  const rootOid = () => session?.routine?.oid ?? selected?.oid;
  const sourceOid = () => (nestedFrame() ? session.frame.oid : rootOid());
  function pointsFor(oid) {
    return oid === rootOid()
      ? points
      : new Set(session?.nested_breakpoints?.find((group) => group.oid === oid)?.lines);
  }
  function source() {
    if (nestedFrame()) return session.frame.source || '';
    return active() || session?.phase === 'finished' ? session.routine?.source || '' : selected?.source || '';
  }
  function renderFrameContext() {
    const frame = session?.phase === 'paused' ? session.frame : null;
    let message = '';
    if (frame) {
      const top = session.stack?.[0]?.level === frame.level;
      message = top && !nestedFrame() ? '' : frame.name;
      if (nestedFrame()) message += frame.source ? ' ' : ' PostgreSQL не вернул исходник этого кадра.';
    }
    $('debug-frame-context').textContent = message;
  }
  let sourcePage = 0,
    sourceText = null,
    sourceLines = [];
  const SOURCE_PAGE_SIZE = 400;
  function renderSource(follow = true) {
    const root = $('debug-source');
    root.replaceChildren();
    renderFrameContext();
    const text = source(),
      frame = session?.phase === 'paused' ? session.frame : null;
    const line =
      frame?.line ?? (session?.position?.oid === session?.routine?.oid ? session?.position?.line : null);
    const inspecting = !!frame && frame.level !== session.stack?.[0]?.level;
    const oid = sourceOid(),
      shownPoints = pointsFor(oid);
    if (text !== sourceText) {
      sourceText = text;
      sourceLines = text.split('\n');
      sourcePage = 0;
    }
    const pause =
      session?.phase === 'paused' ? `${session.session_id}:${session.pause_id}:${frame?.level}` : null;
    if (follow && pause && pause !== shownPause && Number.isInteger(line) && line > 0)
      sourcePage = Math.floor((line - 1) / SOURCE_PAGE_SIZE) * SOURCE_PAGE_SIZE;
    sourcePage = Math.max(
      0,
      Math.min(sourcePage, Math.floor((sourceLines.length - 1) / SOURCE_PAGE_SIZE) * SOURCE_PAGE_SIZE),
    );
    $('debug-source-navigation').hidden = sourceLines.length <= SOURCE_PAGE_SIZE;
    $('source-prev').disabled = sourcePage === 0;
    $('source-next').disabled = sourcePage + SOURCE_PAGE_SIZE >= sourceLines.length;
    $('source-goto').max = sourceLines.length;
    $('source-range').textContent =
      `${sourcePage + 1}-${Math.min(sourcePage + SOURCE_PAGE_SIZE, sourceLines.length)} из ${sourceLines.length}`;
    for (const [index, textLine] of sourceLines.slice(sourcePage, sourcePage + SOURCE_PAGE_SIZE).entries()) {
      if (!text) break;
      const number = sourcePage + index + 1;
      const atPoint = shownPoints.has(number),
        atFrame = line === number && session?.phase === 'paused';
      const row = document.createElement('button');
      row.className =
        'debug-line' +
        (atPoint ? ' breakpoint' : '') +
        (atFrame ? (inspecting ? ' inspected-frame-line' : ' current') : '') +
        (session?.phase === 'finished' && line === number ? ' last-stop' : '');
      row.disabled = pending || (active() && session.phase !== 'paused');
      row.setAttribute(
        'aria-label',
        `${atFrame ? (inspecting ? 'Строка выбранного кадра. ' : 'Следующий оператор. ') : ''}Строка ${number}: ${textLine || 'пусто'}`,
      );
      row.setAttribute('aria-pressed', String(atPoint));
      const marker = document.createElement('span');
      marker.className = 'line-number';
      marker.textContent = (atFrame ? (inspecting ? '◇ ' : '➜ ') : atPoint ? '● ' : '') + number;
      const code = document.createElement('code');
      code.textContent = textLine || ' ';
      row.append(marker, code);
      row.onclick = () =>
        request(async () => {
          const selection = selected,
            sessionId = active() ? session.session_id : null,
            revision = session?.revision;
          $('debug-point-notice').textContent = '';
          const result = await api('/api/debug/resolve', { source: text, line: number });
          if (
            selection !== selected ||
            sessionId !== (active() ? session.session_id : null) ||
            revision !== session?.revision ||
            oid !== sourceOid()
          )
            return;
          if (active()) await changePoints(oid, [result.line], !pointsFor(oid).has(result.line));
          else {
            if (points.has(result.line)) points.delete(result.line);
            else points.add(result.line);
            renderSource();
          }
          if (oid === sourceOid() && result.line !== number)
            $('debug-point-notice').textContent =
              `Строка ${number} относится к оператору на строке ${result.line}.`;
        });
      root.append(row);
    }
    renderStops();
    if (pause && pause !== shownPause && root.getClientRects().length) {
      root.querySelector('.current,.inspected-frame-line')?.scrollIntoView({ block: 'nearest' });
      shownPause = pause;
    }
    if (!pause) shownPause = null;
  }
  $('source-prev').onclick = () => {
    sourcePage -= SOURCE_PAGE_SIZE;
    renderSource(false);
    buttons();
  };
  $('source-next').onclick = () => {
    sourcePage += SOURCE_PAGE_SIZE;
    renderSource(false);
    buttons();
  };
  $('source-go').onclick = () => {
    const value = Number($('source-goto').value);
    if (!Number.isInteger(value) || value < 1 || value > sourceLines.length) {
      $('debug-message').textContent = `Укажите строку от 1 до ${sourceLines.length}.`;
      return;
    }
    sourcePage = Math.floor((value - 1) / SOURCE_PAGE_SIZE) * SOURCE_PAGE_SIZE;
    renderSource(false);
    $('debug-source')
      .querySelectorAll('button')
      [value - sourcePage - 1]?.scrollIntoView({ block: 'nearest' });
    buttons();
  };
  $('source-goto').onkeydown = (event) => {
    if (event.key === 'Enter') {
      $('source-go').click();
    }
  };
  function graphPosition() {
    const valid = !!session?.graph_mapping && stable(draft) === stable(session.project);
    document.dispatchEvent(
      new CustomEvent('debugposition', {
        detail: { block_id: session?.phase === 'paused' && valid ? session.position?.block_id : null },
      }),
    );
  }
  function renderStops() {
    const rootList = $('debug-point-list'),
      nestedList = $('debug-nested-list');
    rootList.replaceChildren();
    nestedList.replaceChildren();
    const groups = [
      {
        oid: rootOid(),
        name: session?.routine?.signature || 'Выбранный объект',
        lines: [...points].sort((a, b) => a - b),
      },
      ...(session?.nested_breakpoints || []),
    ];
    for (const { oid, name, lines } of groups) {
      if (!lines.length) continue;
      const list = oid === rootOid() ? rootList : nestedList;
      const label = document.createElement('span');
      label.textContent = name + ': ';
      list.append(label);
      for (const line of lines) {
        const button = document.createElement('button');
        button.textContent = `Строка ${line} ×`;
        button.title = `Снять точку: ${name}, строка ${line}`;
        button.onclick = () =>
          request(async () => {
            if (active()) await changePoints(oid, [line], false);
            else {
              points.delete(line);
              renderSource();
            }
          });
        list.append(button);
      }
    }
    const stops = session?.stops || [],
      root = $('debug-stops').querySelector('ol');
    root.replaceChildren();
    $('debug-stops').hidden = !stops.length;
    $('debug-stops-summary').textContent =
      `Наблюдавшиеся остановки: ${session?.pause_id || 0}` +
      (stops.length < (session?.pause_id || 0) ? ' (последние 20)' : '');
    for (const stop of stops) {
      const item = document.createElement('li');
      item.value = stop.number;
      item.textContent = `${stop.target}: строка ${stop.line ?? 'не определена'}`;
      root.append(item);
    }
  }
  function renderVariables(data) {
    const rows = comparison.update(data),
      body = $('debug-vars').querySelector('tbody');
    const expanded = new Set(
      Array.from(body.querySelectorAll('details[open]'), (item) => item.dataset.variable),
    );
    body.replaceChildren();
    const show = (v) => (v.value === null ? 'NULL' : v.value) + (v.truncated ? ' …' : '');
    for (const variable of rows) {
      const tr = document.createElement('tr');
      tr.dataset.change = variable.state;
      const name = document.createElement('td'),
        value = document.createElement('td');
      const details = document.createElement('details'),
        title = document.createElement('summary');
      details.dataset.variable = JSON.stringify([
        data.session_id,
        data.frame?.level,
        variable.name,
        variable.declaration_line,
      ]);
      details.open = expanded.has(details.dataset.variable);
      title.textContent = title.title = variable.name;
      const declaration = document.createElement('p');
      declaration.textContent =
        variable.declaration_line === 0 ? 'Параметр' : 'Объявление: строка ' + variable.declaration_line;
      details.append(title, declaration);
      if (variable.previous) {
        const previous = document.createElement('p');
        previous.textContent = 'Было: ' + show(variable.previous);
        details.append(previous);
      }
      if (variable.truncated || variable.state === 'partial') {
        const limitation = document.createElement('p');
        limitation.textContent = 'Показан фрагмент значения.';
        details.append(limitation);
      }
      name.append(details);
      value.textContent = show(variable);
      tr.append(name, value);
      body.append(tr);
    }
  }
  function renderStack(data) {
    const root = $('debug-stack');
    root.replaceChildren();
    for (const frame of data.stack || []) {
      const button = document.createElement('button');
      button.type = 'button';
      button.setAttribute('aria-pressed', String(frame.level === data.frame?.level));
      button.textContent = `#${frame.level} ${frame.name} · строка ${frame.line > 0 ? frame.line : 'не определена'}`;
      if (frame.arguments) {
        const args = document.createElement('small');
        args.textContent = frame.arguments + (frame.truncated ? ' … [показан фрагмент]' : '');
        button.append(args);
      }
      button.onclick = () => request(() => command({ command: 'select_frame', level: frame.level }));
      root.append(button);
    }
  }
  function renderResultTable(result) {
    const section = $('debug-result-table');
    section.hidden = !result;
    if (!result) return;
    const { columns, rows, total_rows, shown_rows, truncated } = result;
    const note = truncated ? ' · Предпросмотр ограничен, сокращённые значения отмечены многоточием.' : '';
    $('debug-table-count').textContent = total_rows
      ? `Показано строк: ${shown_rows} из ${total_rows}${note}`
      : 'Строк результата нет.';
    const head = document.createElement('tr');
    for (const name of columns) {
      const th = document.createElement('th');
      th.scope = 'col';
      th.textContent = name;
      head.append(th);
    }
    section.querySelector('thead').replaceChildren(head);
    const body = document.createDocumentFragment();
    function shownValue(cell) {
      return cell.value === null ? 'NULL' : cell.value + (cell.truncated ? '…' : '');
    }
    for (const row of rows) {
      const tr = document.createElement('tr');
      for (const cell of row) {
        const td = document.createElement('td');
        td.textContent = shownValue(cell);
        if (cell.value === null) td.className = 'debug-sql-null';
        if (cell.truncated) td.title = 'Показан фрагмент значения';
        tr.append(td);
      }
      body.append(tr);
    }
    section.querySelector('tbody').replaceChildren(body);
    section.querySelector('.debug-table-scroll').hidden = !columns.length;
    const copy = $('debug-copy-table'),
      status = $('debug-copy-status');
    copy.disabled = !columns.length;
    status.textContent = '';
    copy.onclick = async () => {
      const field = (value) =>
        value === '' || value === 'NULL' || /[\t\r\n"]/.test(value)
          ? `"${value.replaceAll('"', '""')}"`
          : value;
      const lines = [
        columns.map(field),
        ...rows.map((row) => row.map((cell) => (cell.value === null ? 'NULL' : field(shownValue(cell))))),
      ];
      try {
        await navigator.clipboard.writeText(lines.map((line) => line.join('\t')).join('\n'));
        status.textContent = 'Результат скопирован';
      } catch {
        status.textContent = 'Браузер не разрешил копирование.';
      }
    };
  }
  function renderReport(data) {
    if (data.phase !== 'finished' || !data.outcome) return;
    $('debug-report-target').textContent =
      `${data.database}: ${data.routine?.schema}.${data.routine?.name}(${data.routine?.signature || ''}) · вызов №${data.session_id}`;
    const summary = outcomeSummary(data.outcome),
      root = $('debug-result-summary');
    root.replaceChildren();
    const scalar = data.outcome.scalar;
    const result = data.outcome.result;
    const value =
      scalar != null
        ? 'Результат: ' + scalar
        : result
          ? 'Строк результата: ' + result.total_rows
          : summary.call;
    $('debug-result-short').textContent = data.routine?.name + ' · ' + value + ' · ' + summary.rollback;
    for (const [text, tone] of [
      [summary.call, summary.callTone],
      [summary.rollback, summary.rollbackTone],
      [summary.cancellation, 'warning'],
      [summary.sqlstate ? 'SQLSTATE: ' + summary.sqlstate : null, 'neutral'],
    ]) {
      if (!text) continue;
      const badge = document.createElement('span');
      badge.className = 'outcome-badge';
      badge.dataset.tone = tone;
      badge.textContent = text;
      root.append(badge);
    }
    $('debug-outcome').textContent = data.outcome.text;
    renderResultTable(data.outcome.result);
    $('debug-final-warning').textContent = data.warning ? 'Сообщение во время вызова: ' + data.warning : '';
  }
  function update(data) {
    if (!newerSnapshot(session, data)) return;
    const newSession = session?.session_id !== data.session_id;
    if (
      newSession ||
      session.pause_id !== data.pause_id ||
      session.frame?.level !== data.frame?.level ||
      data.phase === 'finished'
    )
      $('debug-point-notice').textContent = '';
    session = data;
    points = new Set(data.breakpoints || []);
    panel.hidden = false;
    if (newSession && active()) {
      report.close();
      $('debug-view').scrollTop = 0;
    }
    document.dispatchEvent(new CustomEvent('debugsnapshot', { detail: data }));
    if (data.routine) $('debug-title').textContent = data.routine.schema + '.' + data.routine.name;
    $('debug-phase').textContent =
      {
        starting: 'Запуск',
        running: 'Выполняется',
        paused: 'Пауза',
        stopping: 'Ожидаем отмену и откат',
        finished: 'Завершено',
      }[data.phase] || data.phase;
    $('debug-message').textContent = data.warning || '';
    renderVariables(data);
    renderStack(data);
    renderReport(data);
    renderSource();
    graphPosition();
    buttons();
  }
  async function command(action) {
    const id = session.session_id,
      revision = session.revision;
    const data = await api('/api/debug/command', { session_id: id, revision, action });
    if (session?.session_id === id) update(data);
  }
  async function changePoints(oid, lines, enabled) {
    if (oid !== rootOid()) await command({ command: 'nested_breakpoints', oid, lines, enabled });
    else for (const line of lines) await command({ command: 'breakpoint', line, enabled });
  }
  document.addEventListener('constructorchange', (event) => {
    draft = event.detail;
    graphPosition();
  });
  document.addEventListener('workspaceview', (event) => {
    if (event.detail.view === 'debug' && session?.phase === 'paused') {
      shownPause = null;
      renderSource();
    }
  });
  document.addEventListener('catalogchange', (event) => {
    catalogGeneration++;
    if (active()) return;
    report.close();
    catalog = event.detail;
    selected = catalog?.selected || null;
    session = null;
    sourcePage = 0;
    shownPause = null;
    points = new Set();
    panel.hidden = !selected;
    for (const id of ['debug-readiness', 'debug-scenario-panel', 'debug-nested-points']) $(id).close();
    if (selected) {
      renderReadiness();
      const name = catalog.routines.find((r) => r.oid === selected.oid)?.name;
      $('debug-title').textContent = name ? catalog.schema + '.' + name : 'Отладка';
      $('debug-phase').textContent = 'Готово';
      $('debug-message').textContent = blockingReason(catalog);
      argumentForm();
      $('debug-vars').querySelector('tbody').replaceChildren();
      $('debug-stack').replaceChildren();
      renderSource();
    }
    nestedPlan.setCatalog(catalog);
    triggerCall.setCatalog(catalog);
    $('debug-scenario-name').value = '';
    renderScenarios();
    $('debug-point-notice').textContent = '';
    graphPosition();
    buttons();
  });
  document.addEventListener('demoready', (event) => {
    if (!selected || active() || catalog?.database !== event.detail.database) return;
    const input = byIndex('argument', 0);
    if (input) input.value = event.detail.argument;
    buttons();
    $('debug-start').focus();
  });
  $('debug-start').onclick = () =>
    request(async () => {
      let invalid = null;
      for (const [index, arg] of selected.arguments.entries()) {
        if (['IN', 'INOUT'].includes(arg.mode) && validateArgument(index) && invalid === null)
          invalid = index;
      }
      if (invalid !== null) {
        $('debug-message').textContent = 'Исправьте отмеченные аргументы. Вызов не запускался.';
        const input = byIndex('argument', invalid);
        (input?.disabled ? byIndex('default', invalid) : input)?.focus();
        return;
      }
      const data = await api('/api/debug/start', {
        connection_id: catalog.connection_id,
        database: catalog.database,
        schema: catalog.schema,
        routine_oid: selected.oid,
        expected_definition: selected.definition,
        record_fields: selected.arguments.map((arg) => arg.fields || null),
        values: values(),
        default_arguments: [...$('debug-arguments').querySelectorAll('[data-default]:checked')].map((input) =>
          Number(input.dataset.default),
        ),
        breakpoints: [...points],
        nested_breakpoints: nestedPlan.payload(),
        trigger: triggerCall.payload(),
        project: draft,
      });
      update(data);
    });
  $('debug-into').onclick = () => request(() => command({ command: 'step_into' }));
  $('debug-out').onclick = () => request(() => command({ command: 'step_out' }));
  $('debug-next').onclick = () => request(() => command({ command: 'next' }));
  $('debug-continue').onclick = () => request(() => command({ command: 'continue' }));
  $('debug-stop').onclick = async () => {
    if (!active() || stopPending) return;
    stopPending = true;
    buttons();
    try {
      await command({ command: 'stop' });
    } catch (error) {
      $('debug-message').textContent = error.message;
    } finally {
      stopPending = false;
      buttons();
    }
  };
  $('debug-clear').onclick = () =>
    request(async () => {
      const oid = sourceOid();
      if (active()) await changePoints(oid, [...pointsFor(oid)], false);
      else {
        points.clear();
        renderSource();
      }
    });
  // Отмену можно отправить, пока идёт команда. Старые ответы пропускаем.
  setInterval(async () => {
    if (!active() || polling) return;
    const id = session.session_id;
    polling = true;
    try {
      const data = await api('/api/debug/poll', { session_id: id, revision: session.revision });
      if (session?.session_id === id) {
        network();
        update(data);
      }
    } catch (error) {
      if (session?.session_id === id && active())
        network('Связь с сервером: ' + error.message + ' Без связи 15 секунд вызов будет отменён.');
    } finally {
      polling = false;
    }
  }, 500);
  // После обновления страницы возвращаем управление текущим вызовом.
  const restoreGeneration = catalogGeneration;
  api('/api/debug/current', {})
    .then((data) => {
      if (data.phase !== 'idle' && catalogGeneration === restoreGeneration && !session) update(data);
    })
    .catch((error) => network('Сервер недоступен: ' + error.message));
  buttons();
}
