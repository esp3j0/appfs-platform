import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  emoji: string;
  description: ReactNode;
  to: string;
};

const FeatureList: FeatureItem[] = [
  {
    title: '教程',
    emoji: '🎓',
    description: <>手把手、线性、可复现——从挂载 AppFS 到跑通第一条 Agent 消息，含应用开发者与贡献者两条路径。</>,
    to: '/docs/tutorials',
  },
  {
    title: '实操指南',
    emoji: '🛠️',
    description: <>面向具体目标的步骤清单：配置多 Agent、接 Tinode、跑冒烟测试。</>,
    to: '/docs/how-to',
  },
  {
    title: '参考',
    emoji: '📚',
    description: <>面向查阅的契约与字段：动作契约、运行时清单、事件流格式。</>,
    to: '/docs/reference',
  },
  {
    title: '原理',
    emoji: '💡',
    description: <>面向理解的设计动机与权衡：两层为何分离、principal/attach 租约为何这样设计。</>,
    to: '/docs/explanation',
  },
];

function Feature({title, emoji, description, to}: FeatureItem) {
  return (
    <div className={clsx('col col--4')}>
      <Link to={to} className={styles.featureLink}>
        <div className="text--center padding-horiz--md">
          <div className={styles.featureEmoji}>{emoji}</div>
          <Heading as="h3">{title}</Heading>
          <p>{description}</p>
        </div>
      </Link>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="text--center margin-bottom--lg">
          <Heading as="h2">从哪开始？</Heading>
          <p>
            第一次来？<strong>教程</strong>是起点。下面的四个入口按你的目的分：
            <strong>学</strong>用教程、<strong>做</strong>用实操指南、<strong>查</strong>用参考、<strong>懂</strong>用原理。
          </p>
        </div>
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
