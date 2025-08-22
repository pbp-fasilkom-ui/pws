import http from "k6/http";
import { sleep } from "k6";
import execution from "k6/execution";
import exec from "k6/x/exec";

// Read user data from CSV
const csvData = open("user.csv").trim();
const csvLines = csvData.split(/\r?\n/).slice(1); // Skip header
const users = csvLines.map(line => {
  const [username, password] = line.split(",");
  return { username: username.trim(), password: password.trim() };
});

export let options = {
  setupTimeout: "60m",
  teardownTimeout: "60m",
  scenarios: {
    brutal_load: {
      executor: "shared-iterations",
      vus: 200, // 200 concurrent users
      iterations: 200, // Each user pushes once
      maxDuration: "60m",
    },
  },
};

const domain = "http://localhost:8080";

export function setup() {
  console.log({ domain });

  const userCookieMap = {};
  const projectIdMap = {};
  const gitCredsMap = {};

  console.log(`Starting login for ${users.length} users...`);

  // Login all users and store cookies
  for (const user of users) {
    console.log(`Logging in user: ${user.username}`);
    
    const loginRes = http.post(
      domain + "/api/login",
      JSON.stringify({
        username: user.username,
        password: user.password,
      }),
      { headers: { "Content-Type": "application/json" } },
    );

    console.log({
      user: user.username,
      loginStatus: loginRes.status,
    });

    if (loginRes.status !== 302) {
      console.log(`Login failed for ${user.username}`);
      continue;
    }

    const cookies = loginRes.cookies;
    const cookieString = Object.keys(cookies)
      .map((name) => {
        return `${name}=${cookies[name][0].value}`;
      })
      .join("; ");

    userCookieMap[user.username] = cookieString;

    const projectName = "lookandlearn";

    // Create project via API
    const createRes = http.post(
      domain + "/api/project/new",
      JSON.stringify({ owner: user.username, project: projectName }),
      {
        headers: {
          Cookie: cookieString,
          "Content-Type": "application/json",
        },
      },
    );
    
    const parsedRes = JSON.parse(createRes.body);
    if (parsedRes.id) {
      projectIdMap[user.username] = parsedRes.id;
      gitCredsMap[user.username] = {
        username: parsedRes.git_username,
        password: parsedRes.git_password
      };
      console.log(`✓ Created project for ${user.username}: ${parsedRes.id}`);
    } else {
      console.log(`✗ Failed to create project for ${user.username}`);
    }

    // Small delay to avoid overwhelming
    sleep(0.1);
  }

  console.log(`Setup completed. ${Object.keys(userCookieMap).length} users logged in.`);
  return { userCookieMap, projectIdMap, gitCredsMap };
}

export default function({ userCookieMap, projectIdMap, gitCredsMap }) {
  const userIndex = execution.scenario.iterationInTest;
  const user = users[userIndex];
  
  if (!user) {
    console.log(`No user data for iteration ${userIndex}`);
    return;
  }

  const cookieString = userCookieMap[user.username];
  if (!cookieString) {
    console.log(`No cookie found for ${user.username}`);
    return;
  }

  const gitCreds = gitCredsMap[user.username];
  if (!gitCreds) {
    console.log(`No git credentials found for ${user.username}`);
    return;
  }

  const projectName = "lookandlearn";

  console.log(`Starting push simulation for ${user.username}...`);

  // Real git push using exec extension with existing cloned repos
  try {
    console.log(`Executing git push for ${user.username}...`);
    

    
    // Use shell script for git operations since exec extension doesn't work properly
    const gitPushResult = exec.command("bash", [
      "./git-push-single.sh",
      user.username,
      gitCreds.username,
      gitCreds.password,
      projectName
    ]);
    
    console.log(`Git push script output: ${gitPushResult.stdout}`);
    if (gitPushResult.stderr) {
      console.log(`Git push script errors: ${gitPushResult.stderr}`);
    }
    
    if (gitPushResult.exit_code === 0) {
      console.log(`✓ Git push successful for ${user.username}`);
    } else {
      console.log(`✗ Git push failed for ${user.username} with exit code: ${gitPushResult.exit_code}`);
      return;
    }

    // Simple monitoring - check deployment status via API
    let attempts = 0;
    const maxAttempts = 60; // 5 minutes max wait
    
    while (attempts < maxAttempts) {
      sleep(5); // Wait 5 seconds between checks
      
      const statusRes = http.get(
        domain + `/api/project/${user.username}/${projectName}/status`,
        {
          headers: {
            Cookie: cookieString,
          },
        }
      );

      if (statusRes.status === 200) {
        const status = JSON.parse(statusRes.body);
        console.log(`Build status for ${user.username}: ${status.status || 'unknown'}`);
        
        if (status.status === 'SUCCESSFUL' || status.status === 'FAILED') {
          console.log(`✓ Build completed for ${user.username}: ${status.status}`);
          break;
        }
      }
      
      attempts++;
    }

    if (attempts >= maxAttempts) {
      console.log(`⚠ Timeout waiting for build completion: ${user.username}`);
    }

  } catch (error) {
    console.log(`✗ Error during git push for ${user.username}:`, error);
  }
}

export function teardown({ userCookieMap }) {

  console.log("Brutal load test completed!");
}
