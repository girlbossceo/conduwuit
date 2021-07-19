const puppeteer = require('puppeteer');

run().then(() => console.log('Done')).catch(error => {
    console.error("Registration test failed.");
    console.error("There might be a screenshot of the failure in the artifacts.\n");
    console.error(error);
    process.exit(111);
});

async function run() {

    const elementUrl = process.argv[process.argv.length - 2];
    console.debug("Testing registration with ElementWeb hosted at "+ elementUrl);

    const homeserverUrl = process.argv[process.argv.length - 1];
    console.debug("Homeserver url: "+ homeserverUrl);

    const username = "testuser" + String(Math.floor(Math.random() * 100000));
    const password = "testpassword" + String(Math.floor(Math.random() * 100000));
    console.debug("Testuser for this run:\n  User: " + username + "\n  Password: " + password);

    const browser = await puppeteer.launch({
        headless: true, args: [
            "--no-sandbox"
        ]
    });

    const page = await browser.newPage();
    await page.goto(elementUrl);

    await page.screenshot({ path: '01-element-web-opened.png' });

    console.debug("Click [Create Account] button");
    await page.waitForSelector('a.mx_ButtonCreateAccount');
    await page.click('a.mx_ButtonCreateAccount');

    await page.screenshot({ path: '02-clicked-create-account-button.png' });

    // The webapp should have loaded right now, if anything takes more than 5 seconds, something probably broke
    page.setDefaultTimeout(5000);

    console.debug("Click [Edit] to switch homeserver");
    await page.waitForSelector('div.mx_ServerPicker_change');
    await page.click('div.mx_ServerPicker_change');

    await page.screenshot({ path: '03-clicked-edit-homeserver-button.png' });

    console.debug("Type in local homeserver url");
    await page.waitForSelector('input#mx_homeserverInput');
    await page.click('input#mx_homeserverInput');
    await page.click('input#mx_homeserverInput');
    await page.keyboard.type(homeserverUrl);

    await page.screenshot({ path: '04-typed-in-homeserver.png' });

    console.debug("[Continue] with changed homeserver");
    await page.waitForSelector("div.mx_ServerPickerDialog_continue");
    await page.click('div.mx_ServerPickerDialog_continue');

    await page.screenshot({ path: '05-back-to-enter-user-credentials.png' });

    console.debug("Type in username");
    await page.waitForSelector("input#mx_RegistrationForm_username");
    await page.click('input#mx_RegistrationForm_username');
    await page.keyboard.type(username);

    await page.screenshot({ path: '06-typed-in-username.png' });

    console.debug("Type in password");
    await page.waitForSelector("input#mx_RegistrationForm_password");
    await page.click('input#mx_RegistrationForm_password');
    await page.keyboard.type(password);

    await page.screenshot({ path: '07-typed-in-password-once.png' });

    console.debug("Type in password again");
    await page.waitForSelector("input#mx_RegistrationForm_passwordConfirm");
    await page.click('input#mx_RegistrationForm_passwordConfirm');
    await page.keyboard.type(password);

    await page.screenshot({ path: '08-typed-in-password-twice.png' });

    console.debug("Click on [Register] to finish the account creation");
    await page.waitForSelector("input.mx_Login_submit");
    await page.click('input.mx_Login_submit');

    await page.screenshot({ path: '09-clicked-on-register-button.png' });

    // Waiting for the app to login can take some time, so be patient.
    page.setDefaultTimeout(10000);

    console.debug("Wait for chat window to show up");
    await page.waitForSelector("div.mx_HomePage_default_buttons");
    console.debug("Apparently the registration worked.");

    await page.screenshot({ path: '10-logged-in-homescreen.png' });


    // Close the browser and exit the script
    await browser.close();
}