import assert from 'node:assert/strict';
import {messages,locales,applyLocale,t,localizedError} from '../frontend/i18n.js';
assert.deepEqual(Object.keys(locales).sort(),['cs','de','en','fr','it','ja']);
const keys=new Set();
for(const row of messages){assert.equal(row.length,6,row[0]);assert(!keys.has(row[0]),`Duplicate key: ${row[0]}`);keys.add(row[0]);for(const value of row)assert.equal(typeof value==='string'&&value.trim().length>0,true,row[0]);
 const placeholders=s=>[...s.matchAll(/\{(\w+)\}/g)].map(m=>m[1]).sort();for(const value of row)assert.deepEqual(placeholders(value),placeholders(row[0]),row[0]);}
globalThis.document={documentElement:{lang:'',style:{setProperty(){}}}};
for(const locale of Object.keys(locales)){applyLocale(locale);assert(!t('Bitte noch {seconds} Sekunden warten.',{seconds:12}).includes('{'));assert(localizedError('HTTP 429: Zu viele Anfragen. Bitte später erneut versuchen.').startsWith('HTTP 429: '));assert.equal(localizedError('Keine männliche Edge-Stimme für de-DE verfügbar.'),t('Keine männliche Edge-Stimme für {locale} verfügbar.',{locale:'de-DE'}));}
const pdfFitError='Die Übersetzung passt nicht lesbar in das ursprüngliche PDF-Textfeld.';
const pdfFitMessages={
 de:pdfFitError,
 en:'The translation cannot fit legibly into the original PDF text field.',
 cs:'Překlad se do původního textového pole PDF nevejde v čitelné podobě.',
 fr:'La traduction ne tient pas de manière lisible dans le champ de texte PDF d’origine.',
 ja:'翻訳文を元の PDF テキスト領域に読みやすく収めることができません。',
 it:'La traduzione non entra in modo leggibile nel campo di testo PDF originale.'
};
for(const [locale,expected] of Object.entries(pdfFitMessages)){applyLocale(locale);assert.equal(localizedError(pdfFitError),expected,`PDF fit error must use ${locale}`);}
console.log(`Locale catalog passed: ${messages.length} complete messages, six languages, unique keys, matching placeholders.`);
