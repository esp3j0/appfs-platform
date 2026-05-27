import fs from 'node:fs';
import path from 'node:path';
export function registerMountedAppsRoute(app, registry, projectRegistry) {
    app.get('/api/mounted-apps', async (request) => {
        const roots = new Map();
        if (projectRegistry) {
            const projects = request.query.projectId
                ? [projectRegistry.getProject(request.query.projectId)].filter(Boolean)
                : projectRegistry.getProjects();
            for (const project of projects) {
                if (project) {
                    roots.set(path.resolve(project.mountRoot), {
                        mountRoot: project.mountRoot,
                        projectId: project.projectId,
                        projectRoot: project.projectRoot,
                    });
                }
            }
        }
        if (registry.dumpDirectory) {
            roots.set(path.resolve(registry.dumpDirectory), { mountRoot: registry.dumpDirectory });
        }
        const apps = [];
        for (const source of roots.values()) {
            const doc = readMountedAppsDoc(source.mountRoot);
            for (const appDoc of doc.apps) {
                apps.push({
                    ...appDoc,
                    projectId: appDoc.projectId ?? source.projectId,
                    projectRoot: appDoc.projectRoot ?? source.projectRoot,
                });
            }
        }
        return { version: 1, apps };
    });
}
function readMountedAppsDoc(mountRoot) {
    const file = path.join(mountRoot, '_appfs', 'apps.registry.json');
    if (!fs.existsSync(file)) {
        return { version: 1, apps: [] };
    }
    try {
        const content = fs.readFileSync(file, 'utf-8');
        const parsed = JSON.parse(content);
        return {
            version: typeof parsed.version === 'number' ? parsed.version : 1,
            apps: Array.isArray(parsed.apps) ? parsed.apps : [],
        };
    }
    catch {
        return { version: 1, apps: [] };
    }
}
