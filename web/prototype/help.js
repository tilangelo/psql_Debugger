const button=document.getElementById('help-open');
const dialog=document.createElement('dialog');dialog.id='help-dialog';dialog.setAttribute('aria-labelledby','help-title');
const pages={
  start:['Подключение','Выберите «Подключиться к базе» и укажите параметры PostgreSQL.','«Открыть тестовый пример» создаёт отдельную базу. В Linux с --demo используется временный PostgreSQL.'],
  catalog:['Каталог','Выберите базу, схему и объект. Рядом с кодом доступны «Отладить» и «Редактировать».','«Создать в конструкторе» открывает новый проект в выбранной схеме. Если база требует настройки, причина появится над списком.'],
  constructor:['Конструктор','Задайте схему, имя и тип объекта. Добавляйте блоки кнопками + или перетаскиванием.','В SELECT выберите «Собрать из таблиц» или «Ввести SQL». «Проверить в базе» открывает сравнение перед применением.'],
  debug:['Отладка','Введите аргументы и нажмите «Начать отладку». Щёлкните номер строки для точки остановки.','Для точки во вложенной функции до запуска откройте «Ещё» → «Добавить вложенную точку». После остановки доступен стек вызовов. Ошибки запуска и отчёт находятся в «Готовность».'],
  editor:['Редактор тела','Измените PL/pgSQL и нажмите «Проверить тело». Сравните строки и подтвердите запись в окне «Применить изменение?».','Ctrl+F открывает поиск, Ctrl+Space - подсказки SQL.'],
};
dialog.innerHTML=`<form method="dialog"><button class="help-close">Закрыть</button></form><h1 id="help-title"></h1>
<div id="help-current"></div>
<details id="help-more"><summary>Сохранение, откат и выход</summary>
<p>Файлы проекта находятся в меню «Проект». Профили, сценарии, черновики и автокопии сохраняются в текущем браузере. Пароль в профиль не записывается.</p>
<p>Отладочный вызов откатывает транзакционные изменения. Изменения последовательностей и внешние эффекты могут сохраниться. Процедуры с COMMIT или ROLLBACK внутри не поддерживаются.</p>
<p>При потере связи вызов отменяется через 15 секунд. Чтобы остановить программу, нажмите Ctrl+C в терминале.</p></details>`;
document.body.append(dialog);
button.onclick=()=>{
  const view=document.querySelector('.workspace-tabs [aria-selected=true]')?.id.slice(5);
  const [title,...steps]=pages[document.body.classList.contains('onboarding')?'start':view]||pages.start;
  dialog.querySelector('h1').textContent=title;
  dialog.querySelector('#help-current').replaceChildren(...steps.map(step=>{
    const paragraph=document.createElement('p');paragraph.textContent=step;return paragraph;
  }));
  dialog.querySelector('#help-more').open=false;
  dialog.showModal();
};
const version=document.querySelector('meta[name="pg-debugger-version"]')?.content;
if(version)document.querySelector('.brand small').textContent='PostgreSQL · версия '+version;
