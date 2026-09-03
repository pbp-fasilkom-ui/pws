---
sidebar_position: 4
---

# Deploying Your Project
Learn how to deploy your first project in PWS.

:::warning Clone Your Project First

As this is an experimental service, we recommend you to clone your project to a separate folder so it won't disturb your current progress.

:::

## Pushing Changes

1. Go to the project page at `https://stndar.dev/{{ USERNAME }}/{{ PROJECT NAME }}`, or accessing the project you want to deploy from the dashboard.    
       
   ![Projects](./img/projectlist.png)
   ![Projects](./img/projects.png)
2. Copy the command to push. If you want to write it yourself, the format for the command is as follows: 
   ```
    git remote add pws https://stndar.dev/{{ USERNAME }}/{{ PROJECT NAME }}
    git branch -M master
    git push pws master
    ```
   ![Command](./img/command.png)

   :::tip Master Branch

   The documentation uses `master` as the standard deployment branch. PWS also supports other branch names when needed.

   :::

3. Open a terminal on your computer and direct it to your project's directory. For example, `C:/johndoe/projects/bookworm`.
   
4. Ensure your project has the correct configuration. Make sure to read the [prerequisites page](/docs/getting-started/prerequisite) first.

5. In the terminal on your computer, paste the command above to push your project to PWS.

6. In the project page, you can see the status of the project being built and its latest build.    
       
   ![Projects](./img/build.png)

7. Once the status is `Successful`, you can view the deployed application by clicking `Open` or accessing it through the URL format `https://{{ USERNAME }}-{{ PROJECT NAME }}.stndar.dev/`. Make sure to replace `.` with `-`. For example, if your username is `john.doe` and the project is `booker`, then the URL is `https://john-doe-booker.stndar.dev/`.
    
       
   ![Projects](./img/open.png)

8. Congratulations, you have successfully deployed your first web application to PWS!

:::tip Update Changes

   After adding the `pws` remote and setting the branch to `master`, commit and push your changes.
   ```
   git add .
   git commit -m "{{ COMMIT MESSAGE }}"
   git push pws master
    ```
   :::

## Redeploying the Latest Revision

After a project has been pushed at least once, use the **Redeploy** button on the project dashboard to rebuild the latest recorded revision without creating another commit. PWS will add a new build to the queue and open its build log. The button is unavailable while another build for the project is running.
