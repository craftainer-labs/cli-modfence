// Checks the pages `modfence ... --format html` writes, in Chromium: axe
// (WCAG 2.0 to 2.2, A and AA), keyboard use, no JavaScript, and an offline
// load without a Content-Security-Policy violation. The pages are made by
// generate-pages.sh into the directory named by A11Y_PAGES.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;

const DIR = process.env.A11Y_PAGES;
if (!DIR) {
  throw new Error('A11Y_PAGES must name the directory generate-pages.sh wrote to');
}
const PAGES = ['graph', 'graph-symbol', 'graph-grouped', 'trace'];
const TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

/**
 * Opens a page in a fresh context in which the only request allowed is the
 * file itself, and records everything that would show a broken policy.
 */
async function open(browser, name, options = {}) {
  const context = await browser.newContext(options);
  const problems = [];
  const blocked = [];
  await context.route('**/*', (route) => {
    const url = route.request().url();
    if (url.startsWith('file://')) {
      return route.continue();
    }
    blocked.push(url);
    return route.abort();
  });
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      console.error(
        `securitypolicyviolation: ${event.violatedDirective} ${event.blockedURI}`,
      );
    });
  });
  const page = await context.newPage();
  page.on('console', (message) => {
    if (message.type() === 'error') {
      problems.push(`console: ${message.text()}`);
    }
  });
  page.on('pageerror', (error) => problems.push(`pageerror: ${error.message}`));
  await page.goto(pathToFileURL(path.resolve(DIR, `${name}.html`)).href);
  return { context, page, problems, blocked };
}

async function expectNoViolations(page) {
  const results = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  const summary = results.violations.map((violation) => ({
    id: violation.id,
    impact: violation.impact,
    help: violation.help,
    nodes: violation.nodes.map((node) => node.target.join(' ')),
  }));
  expect(summary, JSON.stringify(summary, null, 2)).toEqual([]);
}

/** Presses Tab until the element matching `selector` has the focus. */
async function tabTo(page, selector) {
  for (let step = 0; step < 100; step += 1) {
    await page.keyboard.press('Tab');
    if (await page.evaluate((s) => document.activeElement?.matches(s) ?? false, selector)) {
      return;
    }
  }
  throw new Error(`Tab never reached ${selector}`);
}

const MODES = {
  light: {},
  dark: { colorScheme: 'dark' },
  'reduced motion': { reducedMotion: 'reduce' },
  'dark and reduced motion': { colorScheme: 'dark', reducedMotion: 'reduce' },
};

for (const name of PAGES) {
  test.describe(name, () => {
    for (const [mode, options] of Object.entries(MODES)) {
      test(`axe finds no violation (${mode})`, async ({ browser }) => {
        const { context, page } = await open(browser, name, options);
        await expect(page.locator('#stage')).toBeVisible();
        await expectNoViolations(page);
        await context.close();
      });
    }

    test('axe finds no violation with a node selected', async ({ browser }) => {
      const { context, page } = await open(browser, name, { colorScheme: 'dark' });
      await page.locator('.node-btn').first().click();
      await expect(page.locator('.node-btn[aria-pressed="true"]')).toHaveCount(1);
      await expectNoViolations(page);
      await context.close();
    });

    // axe needs a script context, so it cannot run in a context with
    // JavaScript off. The page's own code is removed instead, which leaves the
    // document a browser without JavaScript shows (the next test checks that
    // with JavaScript really off).
    test('axe finds no violation in the page without its script', async ({ browser }) => {
      const context = await browser.newContext();
      const page = await context.newPage();
      const file = path.resolve(DIR, `${name}.html`);
      // Parsed, not executed (DOMParser runs no script), then every script that
      // is not a JSON data block is removed element by element.
      const { html, executable, data } = await page.evaluate((source) => {
        const doc = new DOMParser().parseFromString(source, 'text/html');
        for (const script of doc.querySelectorAll('script')) {
          if (script.type !== 'application/json') {
            script.remove();
          }
        }
        return {
          html: `<!doctype html>${doc.documentElement.outerHTML}`,
          executable: doc.querySelectorAll('script:not([type="application/json"])').length,
          data: doc.querySelectorAll('script[type="application/json"]').length,
        };
      }, fs.readFileSync(file, 'utf8'));
      expect(executable, 'executable scripts left').toBe(0);
      expect(data, 'the data script survives').toBeGreaterThan(0);
      await page.setContent(html);
      await expect(page.locator('#stage')).toBeHidden();
      await expectNoViolations(page);
      await context.close();
    });

    test('without JavaScript the tables are shown and the diagram is hidden', async ({
      browser,
    }) => {
      const { context, page } = await open(browser, name, { javaScriptEnabled: false });
      await expect(page.locator('#stage')).toBeHidden();
      await expect(page.locator('#controls')).toBeHidden();
      await expect(page.locator('#legend')).toBeHidden();
      await expect(page.getByRole('heading', { name: 'Nodes' })).toBeVisible();
      await expect(page.getByRole('heading', { name: 'Edges' })).toBeVisible();
      await expect(page.locator('table')).toHaveCount(2);
      expect(await page.locator('table').first().locator('tbody tr').count()).toBeGreaterThan(0);
      await context.close();
    });

    test('with JavaScript the diagram and controls work from the keyboard', async ({
      browser,
    }) => {
      const { context, page } = await open(browser, name);
      await expect(page.locator('#stage')).toBeVisible();
      await expect(page.locator('#controls')).toBeVisible();
      await expect(page.locator('#diagram')).toBeVisible();
      expect(await page.locator('#viewport .node').count()).toBeGreaterThan(0);

      const transform = () => page.locator('#viewport').getAttribute('transform');
      let before = await transform();
      for (const id of ['zoom-in', 'pan-up', 'pan-down', 'pan-left', 'pan-right', 'zoom-out']) {
        await tabTo(page, `#${id}`);
        await page.keyboard.press('Enter');
        const after = await transform();
        expect(after, `${id} changes the view`).not.toBe(before);
        before = after;
      }
      await tabTo(page, '#zoom-reset');
      await page.keyboard.press('Space');
      expect(await transform()).toBe('translate(0,0) scale(1)');

      await tabTo(page, '.node-btn');
      const button = page.locator('.node-btn:focus');
      await page.keyboard.press('Enter');
      await expect(button).toHaveAttribute('aria-pressed', 'true');
      await expect(page.locator('#status')).toContainText('selected');
      await expect(page.locator('#viewport .node.sel')).toHaveCount(1);
      await context.close();
    });

    test('filtering by a tag value hides the other table rows and axe still passes', async ({
      browser,
    }) => {
      const { context, page } = await open(browser, name);
      const rows = page.locator('table').first().locator('tbody tr');
      const total = await rows.count();
      await page.selectOption('#tag-key', { index: 1 });
      await expect(page.locator('#tag-value')).toBeEnabled();
      await page.selectOption('#tag-value', { index: 1 });
      const status = await page.locator('#status').textContent();
      const [, matching, all] = status.match(/(\d+) of (\d+) nodes match/);
      expect(Number(all)).toBe(total);
      const visible = page.locator('table').first().locator('tbody tr:not([hidden])');
      await expect(visible).toHaveCount(Number(matching));
      expect(Number(matching)).toBeGreaterThan(0);
      if (name !== 'trace') {
        expect(Number(matching)).toBeLessThan(total);
      }
      await expectNoViolations(page);
      await context.close();
    });

    test('loads offline without a policy violation', async ({ browser }) => {
      const { context, page, problems, blocked } = await open(browser, name);
      await expect(page.locator('#stage')).toBeVisible();
      await page.waitForLoadState('load');
      await page.evaluate(
        () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
      );
      expect(blocked, 'requests other than the page').toEqual([]);
      expect(problems).toEqual([]);
      await context.close();
    });
  });
}
