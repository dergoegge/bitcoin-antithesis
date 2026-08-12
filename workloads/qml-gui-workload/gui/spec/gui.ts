// Specification explored during an Antithesis run.
//
// The default QML specification supplies the action set and the properties that
// apply to any Qt Quick application (no QML errors, never stuck, the process
// stays up). This adds the ones specific to this GUI, in the state this
// workload puts it in: a funded wallet on a chain that keeps moving, which is
// what makes the amount properties below worth asserting at all.

import { always } from "@antithesishq/bombadil";
import {
  actions,
  extract,
  weighted,
  type ActionTemplate,
  type State,
} from "@antithesishq/bombadil/qml";
import {
  clickTargets,
  editableTargets,
  queryAll,
  type ClickTarget,
} from "@antithesishq/bombadil/qml/tree";
import {
  DESTRUCTIVE_PATTERN,
  focusFields,
  keys,
  scrolls,
  typing,
} from "@antithesishq/bombadil/qml/defaults/actions";
export {
  applicationKeepsRunning,
  neverStuck,
  noQmlErrors,
  noSevereMessages,
  pageIsIdentifiable,
} from "@antithesishq/bombadil/qml/defaults/properties";

/**
 * The navigation bar, which is on screen no matter where exploration is.
 *
 * Roughly ten of the clickable items in any state belong to it, so a walk that
 * picks uniformly among targets spends most of its clicks cycling tabs and
 * rarely gets far enough into a page to reach a form, a list row or a dialog. A
 * measured run bore that out: of 1386 clicks, the ten most-clicked targets were
 * all chrome. It still has to be clicked — it is how you change tab — just not
 * as often as everything else put together.
 *
 * The header's node information and warnings buttons count too. They are on
 * every screen for the same reason the tabs are, and de-weighting only the tabs
 * simply moved the pile onto them: they took 86% of the clicks in the run after
 * the first attempt at this.
 */
const CHROME =
  /TabButton$|^walletBadge$|^nodeInformationButton$|^nodeWarningsButton$/;

function isChrome(target: ClickTarget): boolean {
  const { objectName, type } = target.fingerprint;
  return CHROME.test(objectName ?? "") || type === "NetworkIndicator";
}

function isDestructive(target: ClickTarget): boolean {
  const { objectName, text } = target.fingerprint;
  return (
    DESTRUCTIVE_PATTERN.test(objectName ?? "") ||
    DESTRUCTIVE_PATTERN.test(text ?? "")
  );
}

function clickAction(target: ClickTarget): ActionTemplate {
  return { Click: { fingerprint: target.fingerprint, point: target.point } };
}

const targets = extract((state) => clickTargets(state));

const editable = extract((state) => editableTargets(state));

/** Anything that is not the navigation bar: the content of the current page. */
const contentClicks = actions(() =>
  (targets.current ?? [])
    .filter((target) => !isDestructive(target) && !isChrome(target))
    .map(clickAction),
);

/** The navigation bar itself. */
const chromeClicks = actions(() =>
  (targets.current ?? []).filter(isChrome).map(clickAction),
);

// A generator has no literal form, so a fixed string is written as a pattern
// that matches only itself.
const VALID_ADDRESS = {
  Regexp: "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080",
};
const SPENDABLE_AMOUNT = { Regexp: "0[.][0-9]{1,3}" };
const WORD = { Regexp: "[a-z]{3,10}" };

/**
 * What to type into a field, by what the field is for.
 *
 * Typing the same mixture everywhere fills no form: an address field given
 * `-1`, an amount field given an address, and the Send page never reaches a
 * state where its review button is enabled, so everything past the form goes
 * unexplored. Each list is mostly values the field should accept -- and the
 * valid ones are repeated, since one is chosen uniformly -- with the awkward
 * cases mixed in to keep validation under test.
 *
 * Password fields are deliberately absent. Filling both halves of the encrypt
 * dialog correctly would encrypt the wallet, after which the chain drivers
 * cannot spend from it and the rest of the run has no traffic in it.
 */
const FIELD_TEXT: Array<[RegExp, ReadonlyArray<unknown>]> = [
  [
    /[Aa]ddress/,
    [
      VALID_ADDRESS,
      VALID_ADDRESS,
      VALID_ADDRESS,
      // A valid address for the wrong network, and something that is not one.
      { Regexp: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4" },
      { Text: [1, 24] },
    ],
  ],
  [
    /[Aa]mount|[Ff]ee/,
    [
      SPENDABLE_AMOUNT,
      SPENDABLE_AMOUNT,
      SPENDABLE_AMOUNT,
      { Regexp: "21000000" },
      { Regexp: "0[.]00000001" },
      { Regexp: "-1" },
      { Regexp: "9{21}" },
    ],
  ],
  [/[Nn]ote|[Ll]abel|[Mm]essage|[Nn]ame/, [WORD, WORD, WORD, { Text: [1, 24] }]],
  [/[Pp]ath|[Ff]ile/, [{ Regexp: "/tmp/[a-z]{3,8}[.]dat" }, WORD]],
];

/** Everything else: a field whose purpose the name does not give away. */
const OTHER_TEXT = [WORD, SPENDABLE_AMOUNT, { Text: [1, 24] }];

const PASSWORD = /[Pp]assword|[Pp]assphrase/;

function textFor(objectName: string): ReadonlyArray<unknown> {
  if (PASSWORD.test(objectName)) return [];
  for (const [pattern, texts] of FIELD_TEXT) {
    if (pattern.test(objectName)) return texts;
  }
  return OTHER_TEXT;
}

const namedFields = () =>
  (editable.current ?? [])
    .map((target) => target.fingerprint.objectName)
    .filter((objectName): objectName is string => !!objectName);

/**
 * Type into a named field, whether or not it has focus.
 *
 * `objectName` makes the application focus the field before the keys arrive,
 * which is what gets text into this GUI at all: clicking a field here does not
 * focus it, so the focus-dependent path never fires.
 */
const typeIntoFields = actions(() =>
  namedFields().flatMap((objectName) =>
    textFor(objectName).map(
      (text) => ({ TypeText: { text, objectName } }) as ActionTemplate,
    ),
  ),
);

/**
 * Put a value in a field, replacing what was there.
 *
 * Typing appends, which is fine for finding what a field will accept but no way
 * to fill a form: the send page needs a valid address and a valid amount at the
 * same moment, and a field typed into twice holds both values at once. A run
 * left `sendAddressInput` reading
 * `-1-1-1-1$4;½º⁝}۶^0.00000001999999...`, which is not an address, so its
 * review button never enabled and everything past the form went unexplored.
 *
 * This sets the text outright, and the application fires the same edit hooks it
 * would for a real edit, so a form can actually come together.
 */
const fillFields = actions(() =>
  namedFields().flatMap((objectName) =>
    textFor(objectName).map(
      (text) => ({ SetText: { text, objectName } }) as ActionTemplate,
    ),
  ),
);

/**
 * The steps of a payment, offered whenever the interface is ready for one.
 *
 * Sending is several states deep -- reach the tab, put a valid address in one
 * field and a valid amount in another at the same time, review, broadcast --
 * and a walk that picks uniformly gets there by accident or not at all. Four
 * minutes of exploration filled the request-payment form 43 times and the send
 * form never once, because it settles on whichever page it lands on and the
 * navigation bar is deliberately rare.
 *
 * So the flow is offered as a set of actions that are only available where they
 * apply: off the send page this yields the tab, on it the field it still needs,
 * and once the form is good the button that goes forward. It costs nothing in
 * states where none of it applies.
 */
const sendFlow = actions(() => {
  const fields = namedFields();
  const available: ActionTemplate[] = [];
  const clickable = (objectName: string) =>
    (targets.current ?? []).find(
      (target) => target.fingerprint.objectName === objectName,
    );

  if (fields.includes("sendAddressInput")) {
    available.push({
      SetText: { text: VALID_ADDRESS, objectName: "sendAddressInput" },
    } as ActionTemplate);
  }
  if (fields.includes("sendAmountInput")) {
    available.push({
      SetText: { text: SPENDABLE_AMOUNT, objectName: "sendAmountInput" },
    } as ActionTemplate);
  }
  for (const name of [
    "sendReviewButton",
    "sendReviewBroadcastButton",
    "sendResultDoneButton",
  ]) {
    const target = clickable(name);
    if (target) available.push(clickAction(target));
  }
  // Only when there is nothing to do here, so that arriving does not become the
  // whole of what this contributes.
  if (available.length === 0) {
    const tab = clickable("sendTabButton");
    if (tab) available.push(clickAction(tab));
  }
  return available;
});

/**
 * The action mix.
 *
 * The default one weights all clicks together; this splits them so that the
 * ever-present navigation bar competes with the rest of the page rather than
 * drowning it. Everything else is the default's, including its refusal to click
 * controls that quit or reset the application.
 *
 * Typing happens through {@link typeIntoFields}, which names its target, rather
 * than through the default's focus-dependent `typing`. Nothing in this GUI puts
 * focus on a field: a probe clicked `requestPaymentAmountInput` 160 times with
 * real mouse events and focus stayed on the navigation tab in all 165 states,
 * and 90 seconds of exploration with `typing` weighted at 25 produced no typing
 * at all. `typing` is kept in the mix at a low weight for the states where
 * something editable does hold focus, since it costs nothing elsewhere.
 */
export const explore = weighted<ActionTemplate>([
  [20, contentClicks],
  [3, chromeClicks],
  [6, sendFlow],
  [16, fillFields],
  [6, typeIntoFields],
  [4, focusFields],
  [4, typing],
  [4, keys],
  [2, scrolls],
]);

const MAX_SUPPLY_SATS = 2_100_000_000_000_000;

/** The wallet balance shown in the wallet badge. */
const BALANCE_OBJECT_NAME = "walletBadgeBalanceText";

/**
 * Satoshis per unit, for every label the interface puts after an amount.
 *
 * Which one it uses is a display setting, and changing it is a few clicks into
 * exploration, so an amount has to be read together with its unit: `21000000`
 * is the whole supply in BTC and a rounding error in satoshis.
 */
const SATS_PER_UNIT: Record<string, number> = {
  "₿": 100_000_000,
  BTC: 100_000_000,
  mBTC: 100_000,
  bits: 100,
  sat: 1,
  sats: 1,
};

const AMOUNT = /(-?[\d,]+(?:[.,]\d+)?)\s*(mBTC|BTC|bits|sats|sat|₿)(?![A-Za-z])/;

/** Amounts in a piece of displayed text, in satoshis. */
function amountsIn(texts: string[]): number[] {
  return texts
    .map((text) => AMOUNT.exec(text))
    .filter((match): match is RegExpExecArray => match !== null)
    .map(
      (match) =>
        Number(match[1].replaceAll(",", "")) * SATS_PER_UNIT[match[2]],
    )
    .filter((sats) => Number.isFinite(sats));
}

/**
 * Text the interface renders, excluding what has been typed into it.
 *
 * The properties below are about what the GUI puts on screen, and an input
 * field shows whatever the last action put there. Since exploration types
 * random strings, a field can be made to display a `%1` or a number past any
 * bound, and reading those back would be testing the generator rather than the
 * interface: a run reported `noUntranslatedPlaceholders` violated eight times
 * for a `%6` it had typed itself.
 */
function textsOf(state: State): string[] {
  return queryAll(
    state.tree,
    (node) => node.visible && !node.editable && node.text !== null,
  ).map((node) => node.text as string);
}

const staticTexts = extract((state) => textsOf(state));

const displayedAmounts = extract((state) => amountsIn(textsOf(state)));

/** Wallet balances currently on screen, in satoshis. */
const displayedBalances = extract((state) =>
  amountsIn(
    queryAll(
      state.tree,
      (node) =>
        node.visible &&
        node.objectName === BALANCE_OBJECT_NAME &&
        node.text !== null,
    ).map((node) => node.text as string),
  ),
);

/**
 * Translated strings are always substituted.
 *
 * A `%1` reaching the screen means a `qsTr()` call was rendered without its
 * argument, which is invisible to a test that only checks navigation.
 */
export const noUntranslatedPlaceholders = always(() =>
  staticTexts.current.every((text) => !/%[1-9]/.test(text)),
);

/**
 * No amount on screen is larger than the total supply.
 *
 * The magnitude is what is checked, not the sign: this wallet spends, and an
 * outgoing payment is rendered with a leading minus. A number too large in
 * either direction is a conversion or formatting fault, and converting between
 * display units is where one is most likely to come from.
 */
export const amountsWithinSupply = always(() =>
  displayedAmounts.current.every((sats) => Math.abs(sats) <= MAX_SUPPLY_SATS),
);

/**
 * A wallet balance is never negative.
 *
 * Unlike a transaction amount, a balance has no meaningful negative value, so
 * one on screen means the balance was computed or formatted wrongly. The
 * workload keeps this wallet both receiving and spending while it is explored,
 * including while blocks are arriving, which is when a balance is most likely
 * to be assembled from an inconsistent view of the wallet.
 */
export const balancesAreNotNegative = always(() =>
  displayedBalances.current.every((sats) => sats >= 0),
);

/** Every page shows something; a blank page means a load or binding failure. */
export const pagesAreNotBlank = always(() => staticTexts.current.length > 0);

// There was an `overlaysAreDismissable` here, asserting that a visible popup
// always contains something clickable. It is gone because its premise was
// wrong: a Qt Quick popup is dismissed through its `closePolicy` -- a click
// outside it, or Escape -- so an informational one needs no control inside it
// and is not trapping anybody. It reported four shapes of false positive in as
// many runs (a zero-size popup, a menu's divider, a menu's toggles and
// buttons, and finally a bare QQuickPopupItem) and never a real fault.
//
// What it was reaching for is already covered: `neverStuck`, from the default
// properties, requires every state to offer something to click, which is what
// a modal that really trapped exploration would break.
