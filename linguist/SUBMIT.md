# Tulpar-ды GitHub Linguist-ке қосу

`.gitattributes` ішіндегі `linguist-language=Tulpar` **жеке өзі жеткіліксіз**: Linguist тек өз тізіміндегі
тілдердің атын көрсетеді, белгісіз атауды елемейді. "Tulpar" тілі статистикада көріну үшін оны
https://github.com/github-linguist/linguist репозиторийіне pull request арқылы қосу керек.

## Linguist талаптары (CONTRIBUTING.md бойынша, қосар алдында өзіңіз қайта тексеріңіз)

1. Кеңейтім (`.tlp`) GitHub-та көпшілік қолданысында болуы керек: әдетте кемінде ~200 бөлек репозиторий/пайдаланушы.
   Қазір тіл жаңа, сондықтан алдымен репозиторийді жариялап, қолданушылар жинау керек.
2. `lib/linguist/languages.yml` ішіне жазба қосу — `linguist/languages.yml.snippet` дайын үлгі.
3. TextMate грамматикасы (рұқсат етілген лицензиямен): `editors/vscode/syntaxes/tulpar.tmLanguage.json`
   (scope `source.tulpar`). Linguist оны `vendor/grammars` ішіне git submodule ретінде қосады —
   яғни грамматика бөлек публичный репозиторийде болуы керек.
4. `samples/Tulpar/` ішіне нақты .tlp үлгілері (`examples/` және `tests/progs/` файлдарынан алуға болады).
5. `script/add-grammar`, `script/update-ids`, `bundle exec rake test` орындау.

## Қазір не істейді

* `.gitattributes` Linguist-ке `.tlp` — Tulpar деп айтады және оларды "detectable" деп белгілейді.
  Тіл Linguist-ке қосылғанда, бұл файл дәл сол күйінде жұмыс істей бастайды.
* Бұған дейін GitHub `.tlp` файлдарын тілдер статистикасына енгізбейді (жеке репозиторийде де).
