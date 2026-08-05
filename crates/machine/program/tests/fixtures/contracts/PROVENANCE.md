# Contract schema test snapshots

These files are immutable test inputs copied from the private Contracts owner;
they are not Agents-owned production schemas. They keep owner-schema drift
checks reproducible in an isolated Agents checkout without resolving a hidden
`../contracts` path.

| Snapshot | Contracts revision | Blob | Content SHA-256 |
| --- | --- | --- | --- |
| `apxm.port-requirement.v1.json` | `4203656f714cd173388325379c44f8a150d92c15` | `9185b780bf57e5e3a92cdcab85fe9abb3a543cb1` | `99b4327c186023096f3b492bcfab0c0a3668cd592def0ad90a55a8d4ad428e0b` |
| `apxm.contract-common.v1.json` | `950892a924dc7e3bbc24c27eda1861d8bbdc1330` | `fc8cb3f27e0e620d6383b1b138461ee41c965934` | `97f38d2f19b60e964a3bc919ae5a92df76bcdbf673a9462aeb8fa7c1cf98570b` |

The source URLs are:

- `https://github.com/apxm-project/contracts/blob/4203656f714cd173388325379c44f8a150d92c15/schemas/port-requirement.v1.json`
- `https://github.com/apxm-project/contracts/blob/950892a924dc7e3bbc24c27eda1861d8bbdc1330/schemas/contract-common.v1.json`
