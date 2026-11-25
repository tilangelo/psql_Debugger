import { mountDeclarations } from './declarations.js';
import { createHistory } from './history.js';
import { registerHistory, refreshHistory } from './history-ui.js';
import { mountRecovery } from './recovery.js';
import {
  parseWorkingSnapshot,
  encodeWorkingFile,
  parseWorkingFile,
} from './working-draft.js';
import { mountSqlField } from './sql-field.js';
import './catalog.js';
import './workspace.js';
import './body-editor.js';
import './diagnostics.js';
import { block, childBlocks, emptyProject, fitsAtDepth, generate, labels, parseDraft } from './model.js';
import { fillTypeSelect, readTypeChoice } from './data-types.js';
import { requestDatabase } from './database-client.js';
import { mountSelectHelper } from './metadata.js';
import { restoreGeneratedProject } from './reverse-project.js';
const $ = (id) => document.getElementById(id);
let propertyEditors = [],
  flowSignature = null,
  editingBlock = null;
const flowNodes = new Map();
const selectModesByBlock = new WeakMap();
const properties = document.createElement('section');
properties.className = 'block-editor';
properties.setAttribute('aria-label', 'Редактирование блока');
let catalogTypes = [];
let executedBlock = null,
  browserSnapshot = null,
  projectHistory = null,
  projectRecovery = null,
  booting = true,
  restoring = false,
  userTouched = false;
const fingerprint = (value) =>
  JSON.stringify(value, (_, v) =>
    v && typeof v === 'object' && !Array.isArray(v)
      ? Object.keys(v)
          .sort()
          .reduce((result, key) => ((result[key] = v[key]), result), {})
      : v,
  );
const key = 'pg-constructor-draft-v1';
let project = emptyProject(),
  selected;
const importLabel = $('import').parentElement;
importLabel.tabIndex = 0;
importLabel.setAttribute('role', 'button');
importLabel.onkeydown = (event) => {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault();
    $('import').click();
  }
};
let actionStatus = '',
  recoveryStatus = '';
function renderStatus() {
  $('status').textContent = actionStatus || recoveryStatus;
}
function status(message) {
  actionStatus = message;
  renderStatus();
}
function find(id, list = project.blocks, depth = 0) {
  for (const [index, item] of list.entries()) {
    if (item.id === id) return { item, list, index, depth };
    for (const [key] of childBlocks(item)) {
      const found = find(id, item[key], depth + 1);
      if (found) return found;
    }
  }
}

function download(name, text, type) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function button(text, action, cls = '') {
  const b = document.createElement('button');
  b.textContent = text;
  b.className = cls;
  b.onclick = action;
  return b;
}
function workingSnapshot() {
  return {
    project: JSON.parse(JSON.stringify(project)),
    selected: selected || null,
    ...declarationForms.snapshot(),
    branch: $('branch').value,
  };
}
function edited({ group = null } = {}) {
  if (!booting && !restoring) {
    userTouched = true;
    try {
      projectHistory?.record(workingSnapshot(), { group });
    } catch (error) {
      status(error.message);
    }
    projectRecovery?.changed();
  }
  document.dispatchEvent(
    new CustomEvent('constructorchange', { detail: JSON.parse(JSON.stringify(project)) }),
  );
  status('');
  renderFlow();
  renderCode();
  $('title').textContent = project.kind === 'function' ? 'Конструктор функции' : 'Конструктор процедуры';
  $('return-label').hidden = project.kind !== 'function';
  refreshHistory();
}
function load(p, ui = null) {
  if (ui?.selected !== editingBlock) editingBlock = null;
  flowSignature = null;
  project = p;
  selected = ui?.selected || p.blocks[0]?.id;
  if (!find(selected)) selected = p.blocks[0]?.id;
  for (const field of ['schema', 'name', 'kind']) $(field).value = p[field];
  fillTypeSelect($('return-type'), p.returnType, catalogTypes);
  declarationForms.setProject(ui);
  $('branch').value = ui?.branch || 'after';
  edited();
  renderProperties();
}
function restoreSnapshot(value) {
  const next = parseWorkingSnapshot(value);
  restoring = true;
  try {
    load(next.project, next);
  } finally {
    restoring = false;
  }
  userTouched = true;
  projectRecovery?.changed();
  refreshHistory();
}
function historyAction(action) {
  const focus = document.activeElement,
    focusId = focus?.id,
    field = focus?.dataset.workingField;
  const inline = !!focus?.closest('.sql-field-code'),
    blockId = inline ? selected : focus?.dataset.workingBlock;
  const cursor = inline ? propertyEditors[0]?.getCursor() : null,
    start = focus?.selectionStart,
    end = focus?.selectionEnd;
  const next = projectHistory?.[action]();
  if (!next) return;
  if (blockId && find(blockId, next.project.blocks)) next.selected = blockId;
  restoreSnapshot(next);
  if (cursor) propertyEditors[0]?.focusAt(cursor);
  else {
    const input = focusId
      ? $(focusId)
      : field
        ? Array.from(properties.querySelectorAll('[data-working-field]')).find(
            (el) => el.dataset.workingField === field && el.dataset.workingBlock === blockId,
          )
        : null;
    if (input) {
      input.focus();
      if (start != null && typeof input.setSelectionRange === 'function') input.setSelectionRange(start, end);
    }
  }
}
const projectHistoryCommands = { undo: () => historyAction('undo'), redo: () => historyAction('redo') };
let dragged = null;
function insertBlock(list, index, item) {
  list.splice(index, 0, item);
  selected = item.id;
  editingBlock = null;
  edited();
  renderProperties();
}
function makeDraggable(element, payload) {
  element.draggable = true;
  element.addEventListener('dragstart', (event) => {
    dragged = payload.id ? payload : { ...payload, item: block(payload.type) };
    event.dataTransfer.setData('application/x-pgdebug-block', payload.type || payload.id);
    event.dataTransfer.effectAllowed = payload.id ? 'move' : 'copy';
    $('flow').classList.add('dragging');
  });
  element.addEventListener('dragend', clearDrag);
}
function clearDrag() {
  dragged = null;
  $('flow').classList.remove('dragging');
  $('flow')
    .querySelectorAll('.drop-active')
    .forEach((slot) => slot.classList.remove('drop-active'));
}
function chooseBlock(list, index, label, depth) {
  const dialog = $('block-dialog'),
    options = $('block-options');
  $('block-dialog-title').textContent = label;
  options.replaceChildren(
    ...Object.keys(labels).map((type) => {
      const item = block(type);
      const option = button(labels[type], () => {
        dialog.close();
        insertBlock(list, index, item);
        $('flow').querySelector('.node.selected')?.focus();
      });
      option.disabled = !fitsAtDepth(item, depth);
      return option;
    }),
  );
  dialog.showModal();
}
function dropSlot(parent, list, index, ancestors, caption) {
  const label =
    caption ||
    (index === 0
      ? 'Добавить перед первым блоком'
      : index === list.length
        ? 'Добавить в конец'
        : 'Добавить между блоками');
  const slot = button(
    caption ? '＋ ' + caption : '＋',
    () => chooseBlock(list, index, label, ancestors.length),
    'drop-slot',
  );
  slot.title = label;
  slot.setAttribute('aria-label', label);
  function accepts() {
    if (!dragged || ancestors.includes(dragged.id)) return false;
    const item = dragged.id ? find(dragged.id)?.item : dragged.item;
    return !!item && fitsAtDepth(item, ancestors.length);
  }
  slot.addEventListener('dragover', (event) => {
    if (!accepts()) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = dragged.id ? 'move' : 'copy';
    slot.classList.add('drop-active');
  });
  slot.addEventListener('dragleave', () => slot.classList.remove('drop-active'));
  slot.addEventListener('drop', (event) => {
    if (!accepts()) return;
    event.preventDefault();
    const payload = dragged;
    clearDrag();
    if (!payload.id) {
      insertBlock(list, index, payload.item);
      return;
    }
    const found = find(payload.id);
    const destination = list === found.list && found.index < index ? index - 1 : index;
    if (list === found.list && destination === found.index) return;
    found.list.splice(found.index, 1);
    insertBlock(list, destination, found.item);
  });
  parent.append(slot);
}
function blockSummary(b) {
  switch (b.type) {
    case 'assign':
      return b.target + ' := ' + b.expression;
    case 'select':
      return b.query + ' → ' + b.target;
    case 'if':
    case 'while':
      return b.condition;
    case 'for':
      return b.iterator + ': ' + b.lower + ' … ' + b.upper;
    case 'return':
      return project.kind === 'function' ? b.expression : 'Завершить процедуру';
    default:
      return b.sql;
  }
}
function selectBlock(id) {
  if (editingBlock && editingBlock !== id) {
    editingBlock = null;
    renderProperties();
  }
  selected = id;
  $('branch').value = 'after';
  updateFlowNodes();
  renderCode();
}
function toggleEditor(id) {
  selectBlock(id);
  editingBlock = editingBlock === id ? null : id;
  renderProperties();
  updateFlowNodes();
  if (editingBlock) properties.querySelector('input, textarea, .cm-content, button')?.focus();
  else flowNodes.get(id)?.edit.focus();
}
function updateFlowNodes() {
  for (const [id, { node, edit, detail, block }] of flowNodes) {
    node.classList.toggle('selected', id === selected);
    node.classList.toggle('executing', id === executedBlock);
    node.setAttribute('aria-pressed', String(id === selected));
    edit.hidden = id !== selected;
    edit.setAttribute('aria-expanded', String(id === editingBlock));
    edit.textContent = id === editingBlock ? 'Готово' : 'Изменить';
    const summary = blockSummary(block);
    if (detail.textContent !== summary) detail.textContent = summary;
  }
}
function renderFlow() {
  // Оставляем курсор на месте, пока пользователь правит блок.
  const structure = (list) =>
    list.map((b) => [b.id, b.type, ...childBlocks(b).map(([key]) => structure(b[key]))]);
  const signature = JSON.stringify(structure(project.blocks));
  if (signature === flowSignature) {
    updateFlowNodes();
    return;
  }
  flowSignature = signature;
  const root = $('flow');
  root.replaceChildren();
  flowNodes.clear();
  function terminal(text) {
    const element = document.createElement('div');
    element.className = 'terminal';
    element.textContent = text;
    root.append(element);
  }
  terminal('Вход');
  function walk(list, parent, ancestors = [], caption = '') {
    dropSlot(parent, list, 0, ancestors, list.length ? '' : caption || 'Добавить блок');
    for (const [index, b] of list.entries()) {
      const card = document.createElement('div');
      card.className = 'flow-block';
      const header = document.createElement('div');
      header.className = 'flow-block-header';
      const node = button(
        '',
        (event) => {
          if (event.detail === 0) toggleEditor(b.id);
          else selectBlock(b.id);
        },
        'node',
      );
      node.ondblclick = () => toggleEditor(b.id);
      const title = document.createElement('strong');
      title.textContent = labels[b.type];
      const detail = document.createElement('span');
      node.append(title, detail);
      makeDraggable(node, { id: b.id });
      const edit = button('Изменить', () => toggleEditor(b.id), 'edit-block');
      header.append(node, edit);
      card.append(header);
      parent.append(card);
      flowNodes.set(b.id, { node, edit, detail, block: b, card });
      const children = childBlocks(b);
      if (children.length) {
        const branches = document.createElement('div');
        branches.className = children.length === 1 ? 'branches loop-branch' : 'branches';
        for (const [key, text] of children) {
          const section = document.createElement('section');
          const label = document.createElement('h3');
          label.className = 'branch-name';
          label.textContent = text;
          section.append(label);
          walk(
            b[key],
            section,
            [...ancestors, b.id],
            'Добавить в ' + (key === 'body' ? 'тело цикла' : `ветвь «${text}»`),
          );
          branches.append(section);
        }
        parent.append(branches);
      }
      dropSlot(parent, list, index + 1, ancestors);
    }
  }
  walk(project.blocks, root);
  terminal('Завершение');
  if (editingBlock) flowNodes.get(editingBlock)?.card.append(properties);
  updateFlowNodes();
}
function renderCode() {
  const result = generate(project);
  $('errors').textContent = project.name || project.blocks.length ? result.errors.join('\n') : '';
  if (!$('code-pane').open) return;
  $('code').replaceChildren();
  const range = result.sourceMap[selected];
  for (const [i, text] of result.sql.split('\n').entries()) {
    const line = document.createElement('span');
    line.className = 'code-line' + (range && i + 1 >= range.start && i + 1 <= range.end ? ' active' : '');
    line.textContent = text || ' ';
    $('code').append(line);
  }
  $('copy').disabled = !!result.errors.length;
}
function renderProperties() {
  for (const editor of propertyEditors) editor.destroy();
  propertyEditors = [];
  properties.replaceChildren();
  const found = find(editingBlock),
    card = flowNodes.get(editingBlock)?.card;
  if (!found || !card) {
    properties.remove();
    return;
  }
  const b = found.item,
    root = properties;
  card.append(root);
  function field(caption, key, multiline = false) {
    const label = document.createElement('label');
    label.textContent = caption;
    const input = document.createElement(multiline ? 'textarea' : 'input');
    input.value = b[key] || '';
    input.dataset.workingField = key;
    input.dataset.workingBlock = b.id;
    input.spellcheck = false;
    input.oninput = () => {
      b[key] = input.value;
      edited({ group: 'block:' + b.id + ':' + key });
    };
    label.append(input);
    root.append(label);
  }
  if (b.type === 'assign') {
    field('Переменная', 'target');
    field('Выражение', 'expression', true);
  }
  if (b.type === 'select') {
    const tools = document.createElement('div');
    tools.className = 'select-mode-actions';
    tools.setAttribute('role', 'group');
    tools.setAttribute('aria-label', 'Способ составления SELECT');
    const usingTables = button('Собрать из таблиц', () => setMode(true));
    const usingSql = button('Ввести SQL', () => setMode(false));
    tools.append(usingTables, usingSql);
    root.append(tools);
    field('Переменная результата', 'target');
    const tables = document.createElement('div'),
      sql = document.createElement('div');
    root.append(tables, sql);
    let queryEditor;
    const activateTables = mountSelectHelper(
      tables,
      project,
      (query) => {
        b.query = query;
        queryEditor?.setValue(query);
        edited();
      },
      (model) => {
        project.model = model;
        project.schemaVersion = 3;
        edited();
      },
      b,
      (settings) => {
        if (settings === null) delete b.assistant;
        else b.assistant = settings;
        edited();
      },
      (next) => applyTemplate(next),
    );
    queryEditor = mountSqlField(
      sql,
      project,
      b.query,
      'SQL-запрос',
      (value) => {
        b.query = value;
        edited({ group: 'query:' + b.id });
      },
      projectHistoryCommands,
    );
    propertyEditors.push(queryEditor);
    function setMode(fromTables) {
      selectModesByBlock.set(b, fromTables);
      tables.hidden = !fromTables;
      sql.hidden = fromTables;
      usingTables.setAttribute('aria-pressed', String(fromTables));
      usingSql.setAttribute('aria-pressed', String(!fromTables));
      if (fromTables) activateTables();
    }
    setMode(selectModesByBlock.get(b) ?? !!b.assistant);
  }
  if (['if', 'while'].includes(b.type)) field('Логическое выражение', 'condition', true);
  if (b.type === 'for') {
    field('Счётчик цикла', 'iterator');
    field('Нижняя граница', 'lower', true);
    field('Верхняя граница', 'upper', true);
  }
  if (b.type === 'return' && project.kind === 'function') field('Возвращаемое выражение', 'expression', true);
  if (b.type === 'sql') {
    const editor = mountSqlField(
      root,
      project,
      b.sql,
      'SQL внутри блока',
      (value) => {
        b.sql = value;
        edited({ group: 'sql:' + b.id });
      },
      projectHistoryCommands,
    );
    propertyEditors.push(editor);
  }
  const actions = document.createElement('div');
  actions.className = 'actions';
  for (const [caption, delta] of [
    ['Выше', -1],
    ['Ниже', 1],
  ]) {
    const move = button(caption, () => {
      const f = find(selected);
      [f.list[f.index], f.list[f.index + delta]] = [f.list[f.index + delta], f.list[f.index]];
      edited();
      renderProperties();
    });
    move.disabled = found.index + delta < 0 || found.index + delta >= found.list.length;
    actions.append(move);
  }
  actions.append(
    button(
      'Удалить',
      () => {
        const f = find(selected);
        if (
          childBlocks(f.item).some(([key]) => f.item[key].length) &&
          !confirm('Удалить блок вместе с вложенными операциями?')
        )
          return;
        f.list.splice(f.index, 1);
        selected = f.list[Math.min(f.index, f.list.length - 1)]?.id;
        editingBlock = null;
        edited();
        renderProperties();
      },
      'danger',
    ),
  );
  root.append(actions);
}
let constructorSchema = null;
document.addEventListener('catalogchange', (event) => {
  constructorSchema = event.detail?.schema || null;
});
function newProject(schema = constructorSchema || 'public') {
  const current = fingerprint(project);
  const hasDraft =
    userTouched && current !== browserSnapshot && current !== fingerprint(emptyProject(project.schema));
  if (hasDraft && !confirm('Открыть новый проект вместо текущего черновика? Замену можно отменить.')) return;
  load(emptyProject(schema));
  $('view-constructor').click();
  $('name').focus();
}
$('new-project').onclick = () => newProject();
document.addEventListener('constructornew', (event) => newProject(event.detail.schema));
function applyTemplate(next) {
  load(next);
  status('Шаблон открыт');
}
const paletteButtons = [];
for (const type of Object.keys(labels)) {
  const item = button(labels[type], () => {
    $('palette-more').open = false;
    const found = find(selected);
    const destination = $('branch').value;
    const item = block(type);
    const insideBranch = destination !== 'before' && destination !== 'after';
    if (insideBranch && (!found || !childBlocks(found.item).some(([key]) => key === destination))) {
      status(destination === 'body' ? 'Сначала выберите цикл.' : 'Сначала выберите условие IF.');
      return;
    }
    const depth = (found?.depth ?? 0) + Number(insideBranch);
    if (!fitsAtDepth(item, depth)) {
      status('Здесь нельзя добавить ещё один уровень вложенности.');
      return;
    }
    const list = insideBranch ? found.item[destination] : (found?.list ?? project.blocks);
    const index = insideBranch || !found ? list.length : found.index + Number(destination === 'after');
    insertBlock(list, index, item);
  });
  makeDraggable(item, { type });
  paletteButtons.push([type, item]);
}
const narrowPalette = matchMedia('(max-width: 1050px)');
function layoutPalette() {
  const compact = narrowPalette.matches;
  $('palette-more').hidden = !compact;
  if (!compact) $('palette-more').open = false;
  for (const [type, item] of paletteButtons) {
    $(compact && !['assign', 'select', 'if'].includes(type) ? 'palette-extra' : 'palette').append(item);
  }
}
narrowPalette.addEventListener('change', layoutPalette);
layoutPalette();
for (const field of ['schema', 'name', 'kind'])
  $(field).addEventListener('input', () => {
    project[field] = $(field).value;
    edited({ group: field });
    if (field === 'kind') renderProperties();
  });
$('return-type').onchange = () => {
  project.returnType = readTypeChoice($('return-type').value);
  edited();
};
const declarationForms = mountDeclarations({
  roots: { parameters: $('parameters'), variables: $('variables') },
  getProject: () => project,
  getTypes: () => catalogTypes,
  onChange: (key, items, options) => {
    project[key] = items;
    edited(options);
  },
  onBlur: renderProperties,
});
$('save').onclick = () => {
  try {
    parseDraft(JSON.stringify(project));
    localStorage.setItem(key, JSON.stringify(project));
    browserSnapshot = fingerprint(project);
    projectHistory?.breakGroup();
    projectRecovery?.changed();
    projectRecovery?.flush();
    status('Проект сохранён в браузере');
  } catch (e) {
    status('Не сохранено: ' + e.message);
  }
};
$('export').onclick = () => {
  $('project-menu').open = false;
  const name = project.name.replace(/[^a-z_0-9-]/gi, '_') || 'project';
  try {
    let valid = true;
    try {
      parseDraft(JSON.stringify(project));
    } catch {
      valid = false;
    }
    if (valid) {
      download(name + '.pgflow.json', JSON.stringify(project, null, 2), 'application/json');
      status('Проект скачан');
    } else {
      download(name + '.pgwork.json', encodeWorkingFile(workingSnapshot()), 'application/json');
      status(
        'Рабочий черновик выгружен с незавершёнными правками. Перед применением нужно исправить ошибки.',
      );
    }
  } catch (e) {
    status('Не выгружено: ' + e.message);
  }
};
$('import').onchange = async (e) => {
  const file = e.target.files[0];
  if (!file) return;
  $('project-menu').open = false;
  try {
    if (file.size > 2_200_000) throw Error('Файл больше 2,2 МБ.');
    const text = await file.text(),
      working = JSON.parse(text)?.format === 'pgdebug-working-project';
    const next = working ? parseWorkingFile(text) : { project: parseDraft(text) };
    if (
      userTouched &&
      browserSnapshot !== fingerprint(project) &&
      !confirm('Заменить неприменённые правки проектом из файла? Замену можно отменить.')
    )
      return;
    load(next.project, working ? next : null);
    status(
      working ? 'Рабочий черновик открыт. Исправьте ошибки перед применением.' : 'Проект открыт из файла',
    );
  } catch (err) {
    status('Не удалось открыть проект: ' + err.message);
  } finally {
    e.target.value = '';
  }
};
$('copy').onclick = async () => {
  try {
    await navigator.clipboard.writeText(generate(project).sql);
    $('copy-status').textContent = 'Скопировано';
  } catch {
    $('copy-status').textContent = 'Не удалось скопировать. Выделите код вручную.';
  }
};
$('show-code').onclick = () => {
  $('copy-status').textContent = '';
  $('code-pane').showModal();
  renderCode();
};

document.addEventListener('debugposition', (event) => {
  executedBlock = event.detail.block_id;
  renderFlow();
});
document.addEventListener('metadata-context-change', () => {
  if (find(editingBlock)?.item.type === 'select') renderProperties();
});
let selectedCatalog = null;
const typeSource = (value) => value && `${value.connection_id}:${value.database}`;
function refreshTypeChoices() {
  fillTypeSelect($('return-type'), project.returnType, catalogTypes);
  declarationForms.refresh();
}
$('load-types').onclick = async () => {
  if (!selectedCatalog) return;
  const source = typeSource(selectedCatalog),
    schema = $('type-schema').value;
  const action = $('load-types'),
    message = $('type-status');
  action.disabled = true;
  $('type-schema').disabled = true;
  message.textContent = 'Загружаем типы…';
  try {
    const result = await requestDatabase('/api/metadata', {
      connection_id: selectedCatalog.connection_id,
      database: selectedCatalog.database,
      schema,
      relation_oid: null,
      types_only: true,
    });
    if (source !== typeSource(selectedCatalog)) return;
    catalogTypes = [...catalogTypes.filter((type) => type.schema !== schema), ...result.types];
    refreshTypeChoices();
    message.textContent = result.types.length
      ? 'Типы доступны в полях выбора.'
      : 'В схеме нет доступных пользовательских типов.';
  } catch (error) {
    if (source === typeSource(selectedCatalog)) message.textContent = error.message;
  } finally {
    action.disabled = false;
    $('type-schema').disabled = false;
  }
};
document.addEventListener('catalogchange', (event) => {
  const previous = typeSource(selectedCatalog);
  selectedCatalog = event.detail;
  $('catalog-types').hidden = !selectedCatalog;
  const schema = $('type-schema'),
    preferred = previous === typeSource(selectedCatalog) ? schema.value : selectedCatalog?.schema;
  schema.replaceChildren(...(selectedCatalog?.schemas || []).map((name) => new Option(name, name)));
  if (selectedCatalog?.schemas?.includes(preferred)) schema.value = preferred;
  if (previous !== typeSource(selectedCatalog)) {
    catalogTypes = [];
    $('type-status').textContent = '';
    refreshTypeChoices();
  }
  $('restore-graph').disabled = !selectedCatalog?.selected;
  $('restore-graph').hidden = !selectedCatalog?.selected;
  $('restore-graph-status').textContent = '';
  status(selectedCatalog ? `Подключено к ${selectedCatalog.database}.` : 'Подключение к PostgreSQL закрыто.');
});
$('restore-graph').onclick = () => {
  const result = $('restore-graph-status');
  try {
    const next = restoreGeneratedProject(selectedCatalog);
    if (
      userTouched &&
      browserSnapshot !== fingerprint(project) &&
      !confirm('Открыть схему вместо текущего черновика? Замену можно отменить.')
    )
      return;
    load(next);
    result.textContent =
      'Блоки восстановлены в черновике. Настройки полей и связей хранятся в файле проекта.';
  } catch (error) {
    result.textContent = error.message;
  }
};
load(project);
try {
  const saved = localStorage.getItem(key);
  if (saved) {
    const restored = parseDraft(saved);
    browserSnapshot = fingerprint(restored);
    load(restored);
    status('Проект открыт из браузера');
  }
} catch (e) {
  status('Сохранённый черновик не открыт: ' + e.message);
}

booting = false;
projectHistory = createHistory(workingSnapshot());
registerHistory('project', {
  undo: () => historyAction('undo'),
  redo: () => historyAction('redo'),
  available: () => projectHistory.available(),
});
document.addEventListener('focusout', () => projectHistory.breakGroup());
projectRecovery = mountRecovery({
  kind: 'project',
  label: 'проект',
  validate: parseWorkingSnapshot,
  isDirty: () => userTouched && browserSnapshot !== fingerprint(project),
  snapshot: workingSnapshot,
  onStatus: (text) => {
    recoveryStatus = text;
    renderStatus();
  },
  onRestore: (value) => {
    if (
      userTouched &&
      browserSnapshot !== fingerprint(project) &&
      !confirm('Восстановить автокопию вместо текущего черновика? Замену можно отменить.')
    )
      return false;
    load(value.project, value);
    $('start-constructor').click();
    $('view-constructor').click();
    status('Черновик восстановлен');
    return true;
  },
});
